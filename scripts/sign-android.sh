#!/usr/bin/env bash
# Sign and check the Android release packages, with the build tools at the build/pins.env pin
# ($ANDROID_HOME/build-tools/$ANDROID_BUILD_TOOLS_VERSION):
#
#   scripts/sign-android.sh apk IN OUT
#       aligns IN for 16 KB pages (zipalign -P 16) and signs it with apksigner into OUT, with the key in SIGN_KEYSTORE
#       (the keystore, base64), SIGN_KEYSTORE_PASSWORD, SIGN_KEY_ALIAS and SIGN_KEY_PASSWORD; release.yml maps one
#       key's secrets (the GitHub APK's: ANDROID_APK_*) to these. The signature is verified, and its certificate must
#       be packaging/android/apk-certificate.sha256 when that is non-empty, so a secret that isn't the committed key
#       fails here
#   scripts/sign-android.sh apk --throwaway IN OUT
#       the same with a key made for this run (dry runs, which hold no secrets); no certificate is expected
#   scripts/sign-android.sh check APK VERSION_NAME VERSION_CODE [--certificate SHA256 | --unsigned]
#       what android-verify runs: the signature verifies (and is that certificate), or with --unsigned there is none;
#       the APK is aligned for 16 KB pages; every native library's LOAD segments are aligned to at least 16 KB; the
#       manifest carries VERSION_NAME and VERSION_CODE
#
# The keystore goes to a 0600 file in a private temporary directory removed on exit; passwords stay in the environment
# and reach keytool and apksigner through their `env:` forms, never a command line.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
die() { echo "sign-android.sh: $*" >&2; exit 1; }
usage() {
  die "usage: scripts/sign-android.sh apk [--throwaway] IN OUT | check APK VERSION_NAME VERSION_CODE [--certificate SHA256 | --unsigned]"
}

# shellcheck source=../build/pins.env
source "$ROOT/build/pins.env"
TOOLS="${ANDROID_HOME:-$HOME/Android/Sdk}/build-tools/$ANDROID_BUILD_TOOLS_VERSION"
for tool in zipalign apksigner aapt2; do
  [[ -x "$TOOLS/$tool" ]] || die "no $tool in $TOOLS (sdkmanager \"build-tools;$ANDROID_BUILD_TOOLS_VERSION\")"
done
COMMITTED="$ROOT/packaging/android/apk-certificate.sha256"

work="$(umask 077 && mktemp -d)"
trap 'rm -rf "$work"' EXIT

# the SHA-256 of APK's (only) signer certificate, lowercase hex
certificate() {
  local digests
  digests="$("$TOOLS/apksigner" verify --print-certs "$1" | sed -n 's/^Signer #[0-9]* certificate SHA-256 digest: //p')"
  [[ "$(wc -l <<<"$digests")" == 1 && -n "$digests" ]] || die "$1 has no single signer"
  tr '[:upper:]' '[:lower:]' <<<"$digests"
}

sign_apk() {
  local throwaway=false
  [[ "${1:-}" != --throwaway ]] || { throwaway=true; shift; }
  (($# == 2)) || usage
  local in="$1" out="$2" expected=""
  [[ -s "$in" ]] || die "$in is empty or missing"
  if [[ "$throwaway" == true ]]; then
    SIGN_KEYSTORE_PASSWORD="$(openssl rand -hex 24)"
    SIGN_KEY_PASSWORD="$SIGN_KEYSTORE_PASSWORD"
    SIGN_KEY_ALIAS=throwaway
    export SIGN_KEYSTORE_PASSWORD SIGN_KEY_PASSWORD
    keytool -genkeypair -keystore "$work/keystore" -storetype PKCS12 -alias "$SIGN_KEY_ALIAS" -keyalg RSA \
      -keysize 2048 -validity 1 -dname "CN=ngterm throwaway" -storepass:env SIGN_KEYSTORE_PASSWORD \
      -keypass:env SIGN_KEY_PASSWORD >/dev/null 2>&1
  else
    local name
    for name in SIGN_KEYSTORE SIGN_KEYSTORE_PASSWORD SIGN_KEY_ALIAS SIGN_KEY_PASSWORD; do
      [[ -n "${!name:-}" ]] || die "$name is not set"
    done
    (umask 077 && base64 --decode >"$work/keystore" <<<"$SIGN_KEYSTORE") || die "SIGN_KEYSTORE is not base64"
    expected="$(tr -d '[:space:]' <"$COMMITTED" 2>/dev/null || true)"
  fi
  "$TOOLS/zipalign" -P 16 -f 4 "$in" "$work/aligned.apk"
  "$TOOLS/apksigner" sign --ks "$work/keystore" --ks-key-alias "$SIGN_KEY_ALIAS" \
    --ks-pass env:SIGN_KEYSTORE_PASSWORD --key-pass env:SIGN_KEY_PASSWORD --out "$out" "$work/aligned.apk"
  rm -f "$work/keystore"
  local signed
  signed="$(certificate "$out")"
  if [[ -n "$expected" && "$signed" != "$expected" ]]; then
    rm -f "$out"
    die "the APK's certificate is $signed, not the committed $expected (packaging/android/apk-certificate.sha256): the secrets aren't ngterm's APK key"
  fi
  echo "$(basename "$out"): signed and verified (certificate SHA-256 $signed)"
}

check_apk() {
  (($# == 3 || $# == 5 || $# == 4)) || usage
  local apk="$1" name="$2" code="$3" mode="${4:-}" expected="${5:-}"
  [[ -s "$apk" ]] || die "$apk is empty or missing"
  case "$mode" in
    --certificate)
      [[ "$expected" =~ ^[0-9a-f]{64}$ ]] || die "--certificate takes a lowercase SHA-256"
      local signed
      signed="$(certificate "$apk")"
      [[ "$signed" == "$expected" ]] || die "$apk is signed by $signed, not $expected"
      ;;
    --unsigned)
      ! "$TOOLS/apksigner" verify "$apk" >/dev/null 2>&1 || die "$apk is signed; expected unsigned"
      ;;
    "")
      certificate "$apk" >/dev/null
      ;;
    *) usage ;;
  esac
  "$TOOLS/zipalign" -c -P 16 4 "$apk" >/dev/null || die "$apk is not aligned for 16 KB pages (zipalign -c -P 16)"
  mkdir -p "$work/apk"
  unzip -q -o "$apk" 'lib/*' -d "$work/apk" || die "$apk holds no native libraries"
  local library count=0 aligns
  while IFS= read -r -d '' library; do
    count=$((count + 1))
    aligns="$(readelf -lW "$library" | awk '$1 == "LOAD" { print $NF }')"
    [[ -n "$aligns" ]] || die "${library#"$work/apk/"} has no LOAD segment"
    while IFS= read -r align; do
      (($((align)) >= 0x4000)) || die "${library#"$work/apk/"} has a LOAD segment aligned to $align, below 16 KB"
    done <<<"$aligns"
  done < <(find "$work/apk/lib" -name '*.so' -print0)
  ((count > 0)) || die "$apk holds no native libraries"
  local badging
  badging="$("$TOOLS/aapt2" dump badging "$apk" | sed -n '1p')"
  [[ "$badging" == *"versionCode='$code'"* ]] || die "$apk's version code isn't $code: $badging"
  [[ "$badging" == *"versionName='$name'"* ]] || die "$apk's version name isn't $name: $badging"
  echo "$(basename "$apk"): ${mode:-signed}${expected:+ $expected}, aligned for 16 KB pages, $count native libraries with 16 KB LOAD segments, $name ($code)"
}

(($# >= 1)) || usage
command="$1"
shift
case "$command" in
  apk) sign_apk "$@" ;;
  check) check_apk "$@" ;;
  *) usage ;;
esac
