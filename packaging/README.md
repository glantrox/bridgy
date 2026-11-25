# Command distribution

All installation routes expose `bridgy` before handoff setup. They must not run
`bridgy install` as an installation hook. That subcommand edits agent instructions
only when the user explicitly requests handoffs.

## Homebrew

```sh
brew install glantrox/tap/bridgy
```

Run `bridgy` from your project directory. The [public tap](https://github.com/glantrox/homebrew-tap) contains the installable formula. The initial v0.1.0 formula builds from tagged source and Homebrew installs Rust as a build dependency.

`homebrew/bridgy.rb.in` installs the archive's executable with `bin.install "bridgy"`.
Homebrew links it into its own bin directory without a hardcoded prefix. The formula
has no setup hook and tests `--version` plus redirected help. See the official
[Formula Cookbook](https://docs.brew.sh/Formula-Cookbook#bininstall-foo).

After publishing a binary release, unpack `bridgy-packages.tar.gz` and publish
the generated `bridgy.rb` as `Formula/bridgy.rb` in `glantrox/homebrew-tap`.

## WinGet

The three templates under `winget/` generate version, locale, and installer manifests.
The installer is a ZIP with `NestedInstallerType: portable`, containing `bridgy.exe`
with `PortableCommandAlias: bridgy`. WinGet manages the user-scoped executable link;
no handoff setup command runs. This follows Microsoft's
[installer schema](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.10.0/installer.md).

Validate the generated directory on Windows with `winget validate --manifest winget`.
For a local trial, enable local manifests if your WinGet configuration requires it,
then use `winget install --manifest winget`. Submit the validated manifests to the
WinGet catalog to make `winget install --id <PackageIdentifier> --exact` available.
The default identifier is `<GitHub repository owner>.Bridgy`; set the repository
variable `WINGET_PACKAGE_ID` before release to choose a stable identifier.

## Release generation

The release workflow packages four Unix targets as tarballs containing `bridgy`,
retains raw Unix binaries for `install.sh`, and packages Windows x64 as a ZIP
containing `bridgy.exe`. Each artifact has a SHA-256 file. After all builds finish,
the workflow renders package definitions using the actual GitHub repository, tag,
and freshly computed archive hashes. No example release URLs or hashes are shipped
as usable manifests. Templates alone cannot be installed.

To regenerate definitions from downloaded artifacts (Python 3.11+):

```sh
python3 scripts/render_packages.py --repository OWNER/REPOSITORY --tag v0.1.0 --artifacts dist
```

`OWNER/REPOSITORY` must be the real release repository. Generated files go to
`dist/packages/`. Publishing a tagged release does not submit a tap or WinGet PR;
catalog publication remains a maintainer step.

## Acceptance

After installing, open a new terminal outside the source checkout:

```sh
bridgy --version
bridgy
```

The first prints a version; the second opens the session browser, including an empty
view if that directory has no history. Quit with `q`. Provider configuration should
remain untouched until `bridgy install` is explicitly run.

Local installer tests execute the printed PATH commands in isolated homes and start
new interactive/login shells. Windows CI tests the PowerShell copy independently.
The binary Homebrew formula and WinGet manifests require published release artifacts.
