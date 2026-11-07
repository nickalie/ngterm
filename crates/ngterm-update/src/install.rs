use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use semver::Version;

use crate::{Error, Format, Installation, MARKER, error};

/// A verified replacement. The installation is untouched until the app quits or the user chooses to restart.
#[derive(Debug)]
pub struct Prepared {
    pub(crate) installation: Installation,
    pub(crate) work: tempfile::TempDir,
    pub(crate) candidate: PathBuf,
}

impl Prepared {
    /// Starts a detached helper which waits for the app to quit before replacing it, reopening the executable only
    /// when `reopen` is true. A failed replacement restores the previous installation. Errors survive in `error_file` for the
    /// next launch to show. Start this before requesting an orderly quit; if spawning fails, keep the app open.
    ///
    /// # Errors
    /// Returns a protected-path, filesystem or process error when installing is unsafe or the helper cannot start.
    pub fn launch(self, error_file: &Path, reopen: bool, protected_paths: &[PathBuf]) -> Result<(), Error> {
        self.installation.validate_paths(protected_paths)?;
        let original_cwd = std::env::current_dir().unwrap_or_default();
        let script = self
            .work
            .path()
            .join(if cfg!(windows) { "install.ps1" } else { "install.sh" });
        fs::write(&script, if cfg!(windows) { WINDOWS_HELPER } else { UNIX_HELPER })?;
        let mut command = helper_command(&script);
        command
            .arg(std::process::id().to_string())
            .arg(&self.installation.root)
            .arg(&self.candidate)
            .arg(self.work.path())
            .arg(&self.installation.executable)
            .arg(std::path::absolute(error_file)?)
            .arg(if self.installation.format == Format::WindowsMsi {
                "msi"
            } else {
                "tree"
            })
            .current_dir(
                self.work
                    .path()
                    .parent()
                    .ok_or_else(|| error("Update stage has no parent."))?,
            )
            .arg(if reopen { "reopen" } else { "quit" })
            .arg(self.installation.identity_file())
            .arg(if self.installation.format == Format::AppImage {
                "image"
            } else {
                "identity"
            })
            .arg(&original_cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        let mut helper = command.spawn()?;
        if let Err(error) = crate::staging::transfer(self.work.path(), &self.installation, helper.id()) {
            let _ = helper.kill();
            let _ = helper.wait();
            return Err(error);
        }
        // The helper owns its files from here. A failed spawn still drops the TempDir.
        let _ = self.work.keep();
        Ok(())
    }
}

#[cfg(unix)]
fn helper_command(script: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command.arg(script);
    command
}

#[cfg(windows)]
fn helper_command(script: &Path) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script);
    command
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    std::os::unix::process::CommandExt::process_group(command, 0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

pub(crate) fn extract(
    installation: &Installation,
    asset: &Path,
    work: &Path,
    version: &Version,
) -> Result<PathBuf, Error> {
    let unpacked = work.join("unpacked");
    fs::create_dir(&unpacked)?;
    let candidate = match installation.format {
        Format::AppImage => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(asset, fs::Permissions::from_mode(0o755))?;
            }
            return Ok(asset.to_path_buf());
        }
        Format::WindowsMsi => return Ok(asset.to_path_buf()),
        Format::LinuxArchive | Format::MacArchive => {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(asset)?));
            archive.unpack(&unpacked)?;
            if installation.format == Format::LinuxArchive {
                unpacked.join("ngterm")
            } else {
                unpacked
            }
        }
        Format::MacBundle => {
            run(Command::new("/usr/bin/ditto")
                .args(["-x", "-k"])
                .arg(asset)
                .arg(&unpacked))?;
            let candidate = unpacked.join("ngterm.app");
            run(Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&candidate))?;
            candidate
        }
        Format::WindowsArchive => {
            run(Command::new("tar.exe").arg("-xf").arg(asset).arg("-C").arg(&unpacked))?;
            unpacked.join("ngterm")
        }
    };
    validate(&candidate, installation.format, version)?;
    Ok(candidate)
}

fn run(command: &mut Command) -> Result<(), Error> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(error(format!(
            "Release extraction or signature validation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

pub(crate) fn validate(candidate: &Path, format: Format, version: &Version) -> Result<(), Error> {
    let prefix = match format {
        Format::LinuxArchive => candidate.join("usr"),
        Format::MacBundle => candidate.join("Contents"),
        _ => candidate.to_path_buf(),
    };
    let identity = fs::read_to_string(prefix.join("share/ngterm/app-BUILD"))?;
    if identity.lines().next() != Some(version.to_string().as_str()) {
        return Err(error("Downloaded archive identity does not match the release."));
    }
    let marker = if format == Format::MacBundle {
        prefix.join("Resources/update-format")
    } else {
        prefix.join(MARKER)
    };
    if Format::from_marker(&fs::read_to_string(marker)?) != Some(format) {
        return Err(error("Downloaded archive has an incompatible installation format."));
    }
    let bin = if format == Format::MacBundle { "MacOS" } else { "bin" };
    let suffix = if format == Format::WindowsArchive { ".exe" } else { "" };
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"] {
        if !prefix.join(format!("{bin}/{name}{suffix}")).is_file() {
            return Err(error(format!("Downloaded archive has no {name}{suffix}.")));
        }
    }
    Ok(())
}

// Remember the installation before downloading. A different instance or an external installer can update it while
// this app keeps running; the helper refuses to overwrite that replacement with its now stale download.
pub(crate) fn record_installation(installation: &Installation, work: &Path) -> Result<(), Error> {
    if installation.format == Format::AppImage {
        let stamp = installation
            .image_identity
            .as_ref()
            .ok_or_else(|| error("Application image has no recorded identity."))?;
        fs::write(work.join("expected-identity"), stamp)?;
    } else {
        fs::copy(installation.identity_file(), work.join("expected-identity"))?;
    }
    Ok(())
}

pub(crate) fn image_stamp(path: &Path) -> Result<Vec<u8>, Error> {
    let stamp = Command::new("stat").args(["-c", "%d:%i:%s:%Y"]).arg(path).output()?;
    if !stamp.status.success() {
        return Err(error("Could not identify the installed application image."));
    }
    Ok(stamp.stdout)
}

// Arguments are positional, never interpolated into shell code. The helper replaces the complete tree.
pub(crate) const UNIX_HELPER: &str = r#"pid=$1; target=$2; candidate=$3; work=$4; executable=$5; error_file=$6
reopen=$8
original_cwd=${11:-$PWD}
# Neither this helper nor the reopened app may retain a working-directory handle in the old tree.
cd "$work/.." || exit 1
reopen_if_asked() {
  if [ "$reopen" = reopen ]; then
    cd "$original_cwd" 2>/dev/null || cd / || exit 1
    unset APPIMAGE APPDIR OWD
    exec "$executable"
  fi
}
i=0
while kill -0 "$pid" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -ge 300 ]; then
    echo 'The update was cancelled because ngterm did not quit within 60 seconds.' > "$error_file"
    rm -rf "$work"
    exit 1
  fi
  sleep 0.2
done
lock="$target.ngterm-update-lock"
if ! mkdir "$lock"; then
  echo 'Another updater is installing ngterm. This update was not installed.' > "$error_file"
  rm -rf "$work"
  reopen_if_asked
  exit 1
fi
echo "$$" > "$lock/owner.pid"
trap 'rm -f "$lock/owner.pid"; rmdir "$lock" 2>/dev/null' EXIT
if [ "${10}" = image ]; then
  unchanged=false
  actual=$(stat -c '%d:%i:%s:%Y' "$target")
  expected=$(cat "$work/expected-identity")
  [ -n "$actual" ] && [ "$actual" = "$expected" ] && unchanged=true
else
  unchanged=false
  cmp -s "$9" "$work/expected-identity" && unchanged=true
fi
if [ "$unchanged" != true ]; then
  echo 'The installed version changed after the download started. Check for updates again.' > "$error_file"
  rm -f "$lock/owner.pid"; rmdir "$lock"
  trap - EXIT
  rm -rf "$work"
  reopen_if_asked
  exit 1
fi
backup="$work/previous"
if mv "$target" "$backup"; then
  if mv "$candidate" "$target"; then
    rm -f "$error_file"
    rm -rf "$work"
  else
    if mv "$backup" "$target"; then
      echo 'The update could not be installed. The previous installation was restored.' > "$error_file"
    else
      touch "$work/recovery-required"
      echo "The update could not be installed or restored. The previous installation is at $backup." > "$error_file"
    fi
  fi
else
  echo 'The update could not replace the installation. Check directory permissions and try again.' > "$error_file"
fi
if [ ! -e "$backup" ]; then rm -rf "$work"; fi
rm -f "$lock/owner.pid"; rmdir "$lock"
trap - EXIT
reopen_if_asked
"#;

pub(crate) const WINDOWS_HELPER: &str = r#"param($ParentProcess, $Target, $Candidate, $Work, $Executable, $ErrorFile, $Format, $Reopen, $Identity, $IdentityKind, $OriginalDirectory)
$ErrorActionPreference = 'Stop'
if (-not $OriginalDirectory) { $OriginalDirectory = $PWD.Path }
[Environment]::CurrentDirectory = (Split-Path -Parent $Work)
Set-Location -LiteralPath (Split-Path -Parent $Work)
function Clear-Stage {
  if (Test-Path -LiteralPath $Work) {
    Get-ChildItem -LiteralPath $Work -Force | Where-Object Name -ne 'owner.json' | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    $remaining = @(Get-ChildItem -LiteralPath $Work -Force | Where-Object Name -ne 'owner.json')
    if ($remaining.Count -eq 0) {
      Remove-Item -LiteralPath (Join-Path $Work 'owner.json') -Force -ErrorAction SilentlyContinue
      Remove-Item -LiteralPath $Work -Force -ErrorAction SilentlyContinue
    }
  }
}
$lock = "$Target.ngterm-update-lock"
$locked = $false
try {
  $parent = Get-Process -Id $ParentProcess -ErrorAction SilentlyContinue
  if ($parent -and -not $parent.WaitForExit(60000)) { throw 'ngterm did not quit within 60 seconds.' }
  try { New-Item -ItemType Directory -Path $lock -ErrorAction Stop | Out-Null }
  catch { throw "Could not acquire the update lock at $lock. Check for another updater or directory permissions." }
  $locked = $true
  [IO.File]::WriteAllText((Join-Path $lock 'owner.pid'), $PID.ToString())
  $expected = Get-FileHash -LiteralPath (Join-Path $Work 'expected-identity') -Algorithm SHA256
  $actual = Get-FileHash -LiteralPath $Identity -Algorithm SHA256
  if ($expected.Hash -ne $actual.Hash) { throw 'The installed version changed after the download started. Check for updates again.' }
  if ($Format -eq 'msi') {
    $log = Join-Path (Split-Path -Parent $ErrorFile) 'update-installer.log'
    $arguments = '/i "' + $Candidate + '" INSTALLFOLDER="' + $Target + '" /passive /norestart REBOOT=ReallySuppress MSIRESTARTMANAGERCONTROL=DisableShutdown /l*v "' + $log + '"'
    $installer = Start-Process msiexec.exe -ArgumentList $arguments -Wait -PassThru
    if ($installer.ExitCode -eq 3010) { throw "The update was installed, with files pending a Windows restart. Close other ngterm programs or restart Windows before using the updated app. See $log." }
    if ($installer.ExitCode -ne 0) { throw "Windows Installer could not finish the update (code $($installer.ExitCode)). Close other ngterm programs and try again. See $log." }
  } else {
    $backup = Join-Path $Work 'previous'
    try { Move-Item -LiteralPath $Target -Destination $backup }
    catch { throw 'The update could not replace the installation. Check directory permissions and close programs using the installation folder, then try again.' }
    try { Move-Item -LiteralPath $Candidate -Destination $Target }
    catch {
      try { Move-Item -LiteralPath $backup -Destination $Target }
      catch {
        [IO.File]::WriteAllText((Join-Path $Work 'recovery-required'), 'Restore previous installation')
        throw "The update could not be installed or restored. The previous installation is at $backup."
      }
      throw 'The update could not be installed. The previous installation was restored.'
    }
  }
  Remove-Item -LiteralPath $ErrorFile -ErrorAction SilentlyContinue
  Clear-Stage
} catch {
  [IO.File]::WriteAllText($ErrorFile, $_.Exception.Message)
  if ($Format -eq 'msi') {
    Remove-Item -LiteralPath $Candidate -Force -ErrorAction SilentlyContinue
  } elseif (-not (Test-Path -LiteralPath (Join-Path $Work 'previous'))) {
    Clear-Stage
  }
} finally {
  if ($locked) {
    Remove-Item -LiteralPath (Join-Path $lock 'owner.pid') -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $lock -Force -ErrorAction SilentlyContinue
  }
}
if ($Reopen -eq 'reopen') {
  if (-not (Test-Path -LiteralPath $OriginalDirectory -PathType Container)) { $OriginalDirectory = Split-Path -Parent $Work }
  Start-Process -FilePath $Executable -WorkingDirectory $OriginalDirectory
}
"#;

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
