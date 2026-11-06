#!/usr/bin/env bash
# Sign, notarize and package ngterm's macOS layouts under the hardened runtime, and check them.
#
# Every Mach-O but ngterm is a helper, signed first with no entitlements; ngterm, the responsible process for what its
# panes run, is signed last with the entitlements file (packaging/macos/ngterm.entitlements by default). The app is
# signed without --deep, so each nested Mach-O keeps the signature this script gave it. An identity is `-` (ad-hoc,
# with no secure timestamp) or a codesigning identity's SHA-1 hash (with one).
#
#   scripts/sign-macos.sh bundle APP.app [--identity ID] [--entitlements FILE]
#     the app: every Mach-O in the bundle but Contents/MacOS/ngterm, then the app; then a --deep --strict verify;
#   scripts/sign-macos.sh loose BIN [--identity ID] [--entitlements FILE]
#     the archive's loose binaries: every Mach-O in BIN but BIN/ngterm, then BIN/ngterm;
#   scripts/sign-macos.sh check APP.app|BIN [--entitlements FILE] [--signer adhoc|NAME]
#     the layout macos-verify checks: the hardened runtime on every Mach-O, no entitlements on a helper, and exactly
#     the file's entitlements on ngterm; with --signer, every Mach-O signed ad-hoc or by a certificate whose name
#     starts with NAME. It executes nothing it checks;
#   scripts/sign-macos.sh package VERSION OUTPUT_DIR --identity ad-hoc|throwaway|developer-id TREE ARCH [TREE ARCH]...
#     every macOS release package (the .app.zip, DMG, PKG and .tar.gz of ngterm-VERSION-macos-ARCH) from the trees
#     `scripts/package-release.sh --macos-bundle` makes (TREE/ngterm.app and TREE/archive/{bin,share}). It executes
#     nothing from a TREE, so it runs where the signing keys are.
#
# `package`'s identities (the release workflow's macos-sign job picks one):
#   - ad-hoc: the app and the loose binaries as `-` signs them; the DMG and PKG unsigned;
#   - throwaway (dry runs, GitHub-hosted runners only): a self-signed Developer ID Application and Installer pair made
#     here, through the same keychain and signing steps as developer-id, with no notarization. productsign takes only
#     a trusted installer identity, so the Installer certificate is trusted in the runner's admin domain (sudo) until
#     the script ends;
#   - developer-id: the Developer ID pair, notarized and stapled. It reads APPLE_DEVELOPER_ID_APP_P12 and
#     APPLE_DEVELOPER_ID_INSTALLER_P12 (base64 .p12), APPLE_P12_PASSWORD, APPLE_NOTARY_KEY_P8 (base64 App Store
#     Connect API key), APPLE_NOTARY_KEY_ID, APPLE_NOTARY_ISSUER_ID and APPLE_TEAM_ID from the environment, and
#     fails naming any that is missing.
# The order: sign each app, notarize and staple it, then make the .app.zip and DMG from the stapled app and sign the
# DMG, build and productsign the PKG, sign the loose binaries and archive them; last, notarize the DMGs, PKGs and the
# loose binaries (as a zip: a bare Mach-O can't be stapled) together, and staple the DMGs and PKGs. A temporary
# keychain holds the identities; an EXIT trap restores the keychain search list, deletes the keychain, withdraws the
# throwaway trust and removes every key file this script wrote.
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == macos ]] || die "run this script on macOS"
usage() {
  die "usage: scripts/sign-macos.sh bundle APP.app | loose BIN | check APP.app|BIN [--identity ID] [--entitlements FILE] [--signer adhoc|NAME]
  | package VERSION OUTPUT_DIR --identity ad-hoc|throwaway|developer-id TREE ARCH [TREE ARCH]..."
}
SHIPPING_ENTITLEMENTS="$NATIVE_ROOT/packaging/macos/ngterm.entitlements"
# LibreSSL's: `security import` refuses OpenSSL 3's default PKCS#12 encryption
OPENSSL=/usr/bin/openssl

# Every regular file under $1 that is a Mach-O, NUL-separated (links are skipped: they are signed where they point,
# and a release tree holds none that leave it)
mach_o_files() {
  local file kind
  while IFS= read -r -d '' file; do
    kind="$(file -b "$file")"
    [[ "$kind" != Mach-O* ]] || printf '%s\0' "$file"
  done < <(find "$1" -type f -print0)
}

# sign_tree bundle|loose TARGET IDENTITY ENTITLEMENTS
sign_tree() {
  local kind="$1" target="$2" identity="$3" entitlements="$4" main helper file count=0 options
  main="$target/ngterm"
  [[ "$kind" != bundle ]] || main="$target/Contents/MacOS/ngterm"
  [[ -f "$main" ]] || die "no ngterm executable: $main"
  options=(--force --options runtime --sign "$identity")
  [[ "$identity" == - ]] || options+=(--timestamp)
  if [[ "$kind" == bundle ]]; then
    # Contents/MacOS is code: --deep --strict refuses an unsigned file there, and a signature on a script lives in
    # extended attributes, which an archive drops
    while IFS= read -r -d '' file; do
      [[ "$(file -b "$file")" == Mach-O* ]] || die "$file is not a Mach-O: only executables go in Contents/MacOS"
    done < <(find "$target/Contents/MacOS" -type f -print0)
  fi
  while IFS= read -r -d '' helper; do
    [[ "$helper" != "$main" ]] || continue
    codesign "${options[@]}" "$helper"
    count=$((count + 1))
  done < <(mach_o_files "$target")
  if [[ "$kind" == bundle ]]; then
    codesign "${options[@]}" --entitlements "$entitlements" "$target"
    codesign --verify --deep --strict "$target"
  else
    codesign "${options[@]}" --entitlements "$entitlements" "$main"
    codesign --verify --strict "$main"
  fi
  echo "→ signed $target ($count helpers, identity $identity)" >&2
}

# check_entitlements FILE EXPECTED: FILE's signed entitlements equal EXPECTED's (a plist), or are none when EXPECTED is
# empty
check_entitlements() {
  local signed
  signed="$(codesign -d --entitlements - --xml "$1" 2>/dev/null)" || die "$1 is not signed"
  python3 -I - "$1" "$2" "$signed" <<'PY'
import plistlib
import sys

path, expected_file, signed = sys.argv[1:]
actual = plistlib.loads(signed.encode()) if signed.strip() else {}
expected = {}
if expected_file:
    with open(expected_file, "rb") as source:
        expected = plistlib.load(source)
if actual != expected:
    raise SystemExit(f"{path} has entitlements {sorted(actual)}, expected {sorted(expected)}")
PY
}

# has_signer FILE DETAILS SIGNER: codesign -dvv's DETAILS of FILE name SIGNER (adhoc, or a leaf certificate name
# prefix)
has_signer() {
  if [[ "$3" == adhoc ]]; then
    [[ $'\n'"$2"$'\n' == *$'\nSignature=adhoc\n'* ]] || die "$1 is not ad-hoc signed"
  else
    [[ $'\n'"$2" == *$'\nAuthority='"$3"* ]] || die "$1 is not signed by $3"
  fi
}

# check_tree TARGET ENTITLEMENTS [SIGNER]
check_tree() {
  local target="$1" entitlements="$2" signer="${3:-}" main file details count=0
  main="$target/ngterm"
  [[ "$target" != *.app ]] || main="$target/Contents/MacOS/ngterm"
  [[ -f "$main" ]] || die "no ngterm executable: $main"
  while IFS= read -r -d '' file; do
    details="$(codesign -d --verbose=2 "$file" 2>&1)" || die "$file is not signed: $details"
    [[ "$details" =~ flags=0x[0-9a-f]+\([^\)]*runtime ]] || die "$file is signed without the hardened runtime"
    [[ -z "$signer" ]] || has_signer "$file" "$details" "$signer"
    if [[ "$file" == "$main" ]]; then
      check_entitlements "$file" "$entitlements"
    else
      check_entitlements "$file" ""
    fi
    count=$((count + 1))
  done < <(mach_o_files "$target")
  ((count > 0)) || die "no Mach-O under $target"
  echo "→ $target: $count Mach-O files, hardened runtime on each, entitlements on ngterm alone" >&2
}

# ---- package: the keychain, the identities and notarization ----

work=""
keychain=""
search_list=()
trusted_certificate=""
trusted_hash=""
cleanup() {
  if [[ -n "$keychain" ]]; then
    security list-keychains -d user -s "${search_list[@]}" || true
    security delete-keychain "$keychain" || true
  fi
  if [[ -n "$trusted_certificate" ]]; then
    sudo security remove-trusted-cert -d "$trusted_certificate" || true
    sudo security delete-certificate -Z "$trusted_hash" /Library/Keychains/System.keychain || true
  fi
  [[ -z "$work" ]] || rm -rf "$work"
}

# open_keychain: a temporary keychain at the front of the user search list, unlocked for six hours
open_keychain() {
  local password line
  password="$("$OPENSSL" rand -hex 24)"
  keychain="$work/signing.keychain-db"
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line#\"}"
    search_list+=("${line%\"}")
  done < <(security list-keychains -d user)
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  KEYCHAIN_PASSWORD="$password"
  security list-keychains -d user -s "$keychain" "${search_list[@]}"
}

# import_identity P12 PASSWORD: imports it into the keychain and prints its certificate's SHA-1, by which codesign and
# productsign find it (an untrusted identity is not found by name)
import_identity() {
  security import "$1" -k "$keychain" -P "$2" -T /usr/bin/codesign -T /usr/bin/productsign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign:,productsign: -s -k "$KEYCHAIN_PASSWORD" \
    "$keychain" >/dev/null
  P12_PASSWORD="$2" "$OPENSSL" pkcs12 -in "$1" -clcerts -nokeys -passin env:P12_PASSWORD \
    | "$OPENSSL" x509 -noout -fingerprint -sha1 | sed 's/.*=//; s/://g'
}

# throwaway_p12 NAME EXTENSIONS PASSWORD: a self-signed certificate and key as $work/NAME.p12 ($work/NAME.crt beside)
throwaway_p12() {
  cat > "$work/$1.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = ngterm throwaway $1
OU = THROWAWAY0
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
$2
EOF
  "$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days 2 -keyout "$work/$1.key" -out "$work/$1.crt" \
    -config "$work/$1.cnf" 2>/dev/null
  "$OPENSSL" pkcs12 -export -inkey "$work/$1.key" -in "$work/$1.crt" -out "$work/$1.p12" -passout "pass:$3"
}

# decode VARIABLE FILE: the base64 secret in VARIABLE, as FILE
decode() {
  printf '%s' "${!1}" | base64 -D > "$2" || die "$1 is not base64"
  [[ -s "$2" ]] || die "$1 decodes to nothing"
}

# notary_submit FILE: uploads it to the notary service and prints the submission id
notary_submit() {
  local output
  output="$(xcrun notarytool submit "$1" "${NOTARY[@]}" --output-format json)" || die "notarytool submit $1 failed"
  python3 -I -c 'import json, sys; print(json.loads(sys.argv[1])["id"])' "$output"
}

# notary_wait ID FILE: waits for the submission; anything but Accepted prints the notary log and fails
notary_wait() {
  local output status
  output="$(xcrun notarytool wait "$1" "${NOTARY[@]}" --timeout 90m --output-format json)" || true
  status="$(python3 -I -c 'import json, sys; print(json.loads(sys.argv[1]).get("status", ""))' "$output" 2>/dev/null)" \
    || status=""
  if [[ "$status" != Accepted ]]; then
    echo "::error::notarization of $2: ${status:-no status} ($output)" >&2
    xcrun notarytool log "$1" "${NOTARY[@]}" >&2 || true
    exit 1
  fi
  echo "→ notarized $2" >&2
}

staple() {
  xcrun stapler staple "$1"
  xcrun stapler validate "$1"
}

package() {
  (($# >= 6)) || usage
  local version="$1" output="$2" mode tree arch app name team
  shift 2
  [[ "$1" == --identity ]] || usage
  mode="$2"
  shift 2
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected MAJOR.MINOR.PATCH, not $version"
  (($# % 2 == 0)) || usage
  local trees=() arches=()
  while (($#)); do
    [[ "$2" == x86_64 || "$2" == arm64 ]] || die "the macOS arch is x86_64 or arm64, not $2"
    [[ -d "$1/ngterm.app/Contents/MacOS" && -d "$1/archive/bin" && -d "$1/archive/share" ]] \
      || die "not a --macos-bundle tree: $1"
    trees+=("$(cd "$1" && pwd)")
    arches+=("$2")
    shift 2
  done
  mkdir -p "$output"
  output="$(cd "$output" && pwd)"
  work="$(mktemp -d "${TMPDIR:-/tmp}/ngterm-sign.XXXXXX")"
  trap cleanup EXIT

  local app_identity=- installer_identity="" notarize=false password
  NOTARY=()
  case "$mode" in
    ad-hoc) ;;
    throwaway)
      [[ "${RUNNER_ENVIRONMENT:-}" == github-hosted ]] \
        || die "the throwaway identity trusts a certificate system-wide: GitHub-hosted runners only"
      password="$("$OPENSSL" rand -hex 16)"
      throwaway_p12 "Developer ID Application" "extendedKeyUsage = critical,codeSigning" "$password"
      throwaway_p12 "Developer ID Installer" \
        "extendedKeyUsage = critical,1.2.840.113635.100.4.13
1.2.840.113635.100.6.1.14 = critical,DER:05:00" "$password"
      open_keychain
      app_identity="$(import_identity "$work/Developer ID Application.p12" "$password")"
      installer_identity="$(import_identity "$work/Developer ID Installer.p12" "$password")"
      trusted_hash="$installer_identity"
      trusted_certificate="$work/Developer ID Installer.crt"
      sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "$trusted_certificate"
      ;;
    developer-id)
      local variable missing=()
      for variable in APPLE_DEVELOPER_ID_APP_P12 APPLE_DEVELOPER_ID_INSTALLER_P12 APPLE_P12_PASSWORD \
        APPLE_NOTARY_KEY_P8 APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID APPLE_TEAM_ID; do
        [[ -n "${!variable:-}" ]] || missing+=("$variable")
      done
      ((${#missing[@]} == 0)) || die "developer-id signing needs ${missing[*]}"
      decode APPLE_DEVELOPER_ID_APP_P12 "$work/application.p12"
      decode APPLE_DEVELOPER_ID_INSTALLER_P12 "$work/installer.p12"
      decode APPLE_NOTARY_KEY_P8 "$work/notary.p8"
      open_keychain
      app_identity="$(import_identity "$work/application.p12" "$APPLE_P12_PASSWORD")"
      installer_identity="$(import_identity "$work/installer.p12" "$APPLE_P12_PASSWORD")"
      NOTARY=(--key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID")
      notarize=true
      ;;
    *) usage ;;
  esac

  # 2-3: sign each app, then notarize and staple it
  local index ids=()
  for index in "${!trees[@]}"; do
    app="${trees[$index]}/ngterm.app"
    sign_tree bundle "$app" "$app_identity" "$SHIPPING_ENTITLEMENTS"
    if [[ "$mode" == developer-id ]]; then
      team="$(codesign -dv "$app" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
      [[ "$team" == "$APPLE_TEAM_ID" ]] || die "the application identity's team is $team, not APPLE_TEAM_ID $APPLE_TEAM_ID"
    fi
    if [[ "$notarize" == true ]]; then
      ditto -c -k --keepParent "$app" "$work/app-${arches[$index]}.zip"
      ids+=("$(notary_submit "$work/app-${arches[$index]}.zip")")
    fi
  done
  if [[ "$notarize" == true ]]; then
    for index in "${!trees[@]}"; do
      notary_wait "${ids[$index]}" "ngterm.app (${arches[$index]})"
      staple "${trees[$index]}/ngterm.app"
    done
  fi

  # 4-6: the .app.zip and DMG from the stapled app, the PKG, the loose binaries and their archive
  local submissions=() labels=()
  for index in "${!trees[@]}"; do
    tree="${trees[$index]}"
    arch="${arches[$index]}"
    app="$tree/ngterm.app"
    name="$output/ngterm-$version-macos-$arch"
    codesign --verify --deep --strict "$app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$name.app.zip"
    mkdir "$work/dmg-$arch"
    cp -R "$app" "$work/dmg-$arch/"
    ln -s /Applications "$work/dmg-$arch/Applications"
    hdiutil create -volname ngterm -srcfolder "$work/dmg-$arch" -ov -format UDZO "$name.dmg"
    pkgbuild --component "$app" --install-location /Applications --identifier com.nickalie.ngterm \
      --version "$version" "$work/unsigned-$arch.pkg"
    if [[ "$app_identity" == - ]]; then
      mv "$work/unsigned-$arch.pkg" "$name.pkg"
    else
      codesign --force --timestamp --sign "$app_identity" "$name.dmg"
      productsign --sign "$installer_identity" --keychain "$keychain" --timestamp "$work/unsigned-$arch.pkg" "$name.pkg"
    fi
    pkgutil --payload-files "$name.pkg" > "$work/pkg-files-$arch"
    grep -q 'Contents/MacOS/ngterm' "$work/pkg-files-$arch"
    sign_tree loose "$tree/archive/bin" "$app_identity" "$SHIPPING_ENTITLEMENTS"
    tar -C "$tree/archive" -czf "$name.tar.gz" bin share
    if [[ "$notarize" == true ]]; then
      ditto -c -k --keepParent "$tree/archive/bin" "$work/loose-$arch.zip"
      submissions+=("$name.dmg" "$name.pkg" "$work/loose-$arch.zip")
    fi
  done

  # 7: the DMGs, PKGs and loose binaries together; the DMGs and PKGs stapled
  if [[ "$notarize" == true ]]; then
    ids=()
    for name in "${submissions[@]}"; do
      ids+=("$(notary_submit "$name")")
      labels+=("$(basename "$name")")
    done
    for index in "${!ids[@]}"; do
      notary_wait "${ids[$index]}" "${labels[$index]}"
    done
    for name in "${submissions[@]}"; do
      [[ "$name" == *.zip ]] || staple "$name"
    done
  fi
  echo "→ macOS packages in $output (identity $mode)" >&2
}

(($# >= 2)) || usage
command="$1"
if [[ "$command" == package ]]; then
  shift
  package "$@"
  exit 0
fi
target="$2"
shift 2
identity=-
entitlements="$SHIPPING_ENTITLEMENTS"
signer=""
while (($#)); do
  (($# >= 2)) || usage
  case "$1" in
    --identity) identity="$2" ;;
    --entitlements) entitlements="$2" ;;
    --signer) signer="$2" ;;
    *) usage ;;
  esac
  shift 2
done
[[ -f "$entitlements" ]] || die "no entitlements file: $entitlements"
[[ -d "$target" ]] || die "not a directory: $target"
case "$command" in
  bundle)
    [[ "$target" == *.app ]] || die "not an app bundle: $target"
    sign_tree bundle "$target" "$identity" "$entitlements"
    ;;
  loose) sign_tree loose "$target" "$identity" "$entitlements" ;;
  check) check_tree "$target" "$entitlements" "$signer" ;;
  *) usage ;;
esac
