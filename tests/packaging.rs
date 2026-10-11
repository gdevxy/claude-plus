use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn script(name: &str) -> PathBuf {
    root().join("packaging").join(name)
}

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cp-pkg-")
        .tempdir_in("/tmp")
        .unwrap()
}

fn run(command: &mut Command) -> Output {
    command.output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn release(dir: &Path, owner: &str) -> PathBuf {
    let dist = dir.join("dist");
    for target in TARGETS {
        let packaged = run(Command::new("sh")
            .arg(script("package.sh"))
            .args([target, env!("CARGO_BIN_EXE_claude-plus")])
            .arg(&dist));
        assert!(packaged.status.success(), "{}", text(&packaged.stderr));
    }
    let assets = run(Command::new("sh")
        .arg(script("release-assets.sh"))
        .args([VERSION, owner])
        .arg(&dist));
    assert!(assets.status.success(), "{}", text(&assets.stderr));
    dist
}

fn install(script: &Path, home: &Path, vars: &[(&str, &str)]) -> Output {
    run(Command::new("sh")
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .envs(vars.iter().copied()))
}

#[test]
fn the_installer_of_a_release_installs_its_binary_into_a_scratch_home_with_a_space() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let home = temp.path().join("my home");
    let url = format!("file://{}", dist.display());
    let output = install(
        &dist.join("install.sh"),
        &home,
        &[("CLAUDE_PLUS_DOWNLOAD_URL", &url)],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let bin = home.join(".local/bin");
    assert_eq!(
        text(&output.stdout),
        format!(
            "✻ Installed claude-plus {VERSION} to {0}/claude-plus\n  {0} is not on PATH; add it, for example with: export PATH=\"{0}:$PATH\"\n✻ Next, run claude-plus install to add the pane to Claude Code\n",
            bin.display()
        )
    );
    let version = run(Command::new(bin.join("claude-plus")).arg("--version"));
    assert_eq!(text(&version.stdout), format!("claude-plus {VERSION}\n"));
    assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 1);
}

#[test]
fn the_installer_installs_where_it_is_told_and_says_nothing_about_a_path_that_has_it() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let dir = temp.path().join("tools");
    let url = format!("file://{}", dist.display());
    let output = run(Command::new("sh")
        .arg(dist.join("install.sh"))
        .env_clear()
        .env("HOME", temp.path())
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", dir.display()),
        )
        .env("CLAUDE_PLUS_DOWNLOAD_URL", &url)
        .env("CLAUDE_PLUS_INSTALL_DIR", &dir));
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        format!(
            "✻ Installed claude-plus {VERSION} to {}/claude-plus\n✻ Next, run claude-plus install to add the pane to Claude Code\n",
            dir.display()
        )
    );
}

#[test]
fn the_installer_refuses_a_download_that_does_not_match_its_checksum() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS")).unwrap();
    let tampered: String = sums
        .lines()
        .map(|line| format!("{}{}\n", "0".repeat(64), &line[64..]))
        .collect();
    std::fs::write(dist.join("SHA256SUMS"), tampered).unwrap();
    let home = temp.path().join("home");
    let url = format!("file://{}", dist.display());
    let output = install(
        &dist.join("install.sh"),
        &home,
        &[("CLAUDE_PLUS_DOWNLOAD_URL", &url)],
    );
    assert!(!output.status.success());
    assert!(
        text(&output.stderr)
            .ends_with("does not match its SHA256SUMS line, so nothing was installed\n"),
        "{}",
        text(&output.stderr)
    );
    assert!(!home.join(".local/bin/claude-plus").exists());
}

#[test]
fn a_truncated_installer_fails_and_changes_nothing() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let full = std::fs::read_to_string(dist.join("install.sh")).unwrap();
    let guard = full
        .find("trap ")
        .expect("the installer sets a guard first");
    let body_starts = guard + full[guard..].find('\n').unwrap() + 1;
    let mut cuts: Vec<usize> = full[body_starts..]
        .match_indices('\n')
        .map(|(at, _)| body_starts + at + 1)
        .filter(|&at| at < full.len())
        .collect();
    cuts.extend(cuts.clone().iter().map(|at| at - 3));
    cuts.push(full.len() - 3);
    let url = format!("file://{}", dist.display());
    for cut in cuts {
        let run_dir = temp.path().join(format!("cut-{cut}"));
        let (home, tmp) = (run_dir.join("home"), run_dir.join("tmp"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        let truncated = run_dir.join("install.sh");
        std::fs::write(&truncated, &full[..cut]).unwrap();
        let output = install(
            &truncated,
            &home,
            &[
                ("CLAUDE_PLUS_DOWNLOAD_URL", &url),
                ("TMPDIR", tmp.to_str().unwrap()),
            ],
        );
        assert!(
            !output.status.success(),
            "cut at {cut} exited 0 after {:?}",
            &full[cut.saturating_sub(40)..cut]
        );
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0, "cut at {cut}");
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0, "cut at {cut}");
    }
    assert_eq!(full.lines().last(), Some("main \"$@\""));
}

#[test]
fn the_installer_names_a_directory_it_cannot_write_to() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    std::fs::write(temp.path().join("file"), "").unwrap();
    let dir = temp.path().join("file/bin");
    let url = format!("file://{}", dist.display());
    let output = install(
        &dist.join("install.sh"),
        temp.path(),
        &[
            ("CLAUDE_PLUS_DOWNLOAD_URL", &url),
            ("CLAUDE_PLUS_INSTALL_DIR", dir.to_str().unwrap()),
        ],
    );
    assert!(!output.status.success());
    assert_eq!(
        text(&output.stderr),
        format!(
            "claude-plus install.sh: cannot write to {}; set CLAUDE_PLUS_INSTALL_DIR to a directory you can write to\n",
            dir.display()
        )
    );
}

#[test]
fn the_installer_takes_a_version_with_or_without_its_v() {
    use std::os::unix::fs::PermissionsExt;
    let temp = scratch();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("curl"), "#!/bin/sh\nexit 22\n").unwrap();
    std::fs::set_permissions(bin.join("curl"), std::fs::Permissions::from_mode(0o755)).unwrap();
    for version in ["v0.1.0", "0.1.0"] {
        let output = run(Command::new("sh")
            .arg(script("install.sh"))
            .env_clear()
            .env("HOME", temp.path())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("CLAUDE_PLUS_GITHUB_OWNER", "ada")
            .env("CLAUDE_PLUS_VERSION", version));
        assert!(!output.status.success());
        assert!(
            text(&output.stderr).contains(
                "could not download https://github.com/ada/claude-plus/releases/download/v0.1.0/claude-plus-"
            ),
            "{}",
            text(&output.stderr)
        );
    }
}

#[test]
fn the_installer_in_the_repository_asks_for_a_github_owner() {
    let temp = scratch();
    let output = install(&script("install.sh"), temp.path(), &[]);
    assert!(!output.status.success());
    assert_eq!(
        text(&output.stderr),
        "claude-plus install.sh: this copy of install.sh names no GitHub owner; use the install.sh of a claude-plus release, or set CLAUDE_PLUS_GITHUB_OWNER\n"
    );
}

#[test]
fn the_installer_names_a_platform_without_a_build() {
    use std::os::unix::fs::PermissionsExt;
    let temp = scratch();
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo FreeBSD ;; -m) echo amd64 ;; esac\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("uname"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = run(Command::new("sh")
        .arg(script("install.sh"))
        .env_clear()
        .env("HOME", temp.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("CLAUDE_PLUS_GITHUB_OWNER", "ada"));
    assert!(!output.status.success());
    assert_eq!(
        text(&output.stderr),
        "claude-plus install.sh: claude-plus runs on macOS and Linux, not FreeBSD\n"
    );
}

#[test]
fn release_assets_hold_checksums_a_formula_and_an_installer_for_the_owner() {
    let temp = scratch();
    let dist = release(temp.path(), "ada");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS")).unwrap();
    let names: Vec<&str> = sums.lines().map(|line| &line[66..]).collect();
    assert_eq!(
        names,
        TARGETS.map(|target| format!("claude-plus-{target}.tar.gz"))
    );
    let formula = std::fs::read_to_string(dist.join("claude-plus.rb")).unwrap();
    for line in sums.lines() {
        let (sum, archive) = line.split_once("  ").unwrap();
        assert!(formula.contains(&format!(
            "      url \"https://github.com/ada/claude-plus/releases/download/v{VERSION}/{archive}\"\n      sha256 \"{sum}\"\n"
        )), "{formula}");
    }
    assert!(formula.contains(&format!("  version \"{VERSION}\"\n")));
    assert!(!formula.contains('@'), "{formula}");
    let ruby = run(Command::new("ruby")
        .arg("-c")
        .arg(dist.join("claude-plus.rb")));
    assert!(ruby.status.success(), "{}", text(&ruby.stderr));
    let installer = std::fs::read_to_string(dist.join("install.sh")).unwrap();
    assert!(installer.contains("github_owner=\"${CLAUDE_PLUS_GITHUB_OWNER:-ada}\"\n"));
    assert!(!installer.contains("@GITHUB_OWNER@"));
    let parsed = run(Command::new("sh").arg("-n").arg(dist.join("install.sh")));
    assert!(parsed.status.success(), "{}", text(&parsed.stderr));
    let listing = run(Command::new("tar")
        .arg("-tzf")
        .arg(dist.join("claude-plus-x86_64-apple-darwin.tar.gz")));
    assert_eq!(text(&listing.stdout), "claude-plus\nLICENSE\nREADME.md\n");
}

#[test]
fn an_archive_of_the_same_binary_is_the_same_bytes() {
    let temp = scratch();
    let binary = temp.path().join("claude-plus");
    std::fs::copy(env!("CARGO_BIN_EXE_claude-plus"), &binary).unwrap();
    if cfg!(target_os = "macos") {
        let tagged = run(Command::new("xattr")
            .args(["-w", "com.apple.metadata:kMDItemWhereFroms", "test"])
            .arg(&binary));
        assert!(tagged.status.success(), "{}", text(&tagged.stderr));
    }
    let archive = |name: &str| {
        let dist = temp.path().join(name);
        let packaged = run(Command::new("sh")
            .arg(script("package.sh"))
            .arg("aarch64-apple-darwin")
            .arg(&binary)
            .arg(&dist)
            .env("USER", name));
        assert!(packaged.status.success(), "{}", text(&packaged.stderr));
        dist.join("claude-plus-aarch64-apple-darwin.tar.gz")
    };
    let first = archive("first");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = archive("second");
    assert!(
        std::fs::read(&first).unwrap() == std::fs::read(&second).unwrap(),
        "two archives of the same binary differ"
    );
    let listing = run(Command::new("tar").arg("-tvzf").arg(&first));
    let listing = text(&listing.stdout);
    assert_eq!(listing.lines().count(), 3, "{listing}");
    assert!(!listing.contains("._"), "{listing}");
}

#[test]
fn release_assets_refuse_to_leave_a_placeholder_unfilled() {
    let temp = scratch();
    let dist = temp.path().join("dist");
    for target in TARGETS {
        let packaged = run(Command::new("sh")
            .arg(script("package.sh"))
            .args([target, env!("CARGO_BIN_EXE_claude-plus")])
            .arg(&dist));
        assert!(packaged.status.success(), "{}", text(&packaged.stderr));
    }
    let assets = run(Command::new("sh")
        .arg(script("release-assets.sh"))
        .args([VERSION, "@OWNER@"])
        .arg(&dist));
    assert!(!assets.status.success());
    assert!(
        text(&assets.stderr)
            .ends_with("release-assets.sh: a placeholder above was left unfilled\n"),
        "{}",
        text(&assets.stderr)
    );
}

#[test]
fn release_assets_need_all_four_builds() {
    let temp = scratch();
    let dist = temp.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    let packaged = run(Command::new("sh")
        .arg(script("package.sh"))
        .args(["aarch64-apple-darwin", env!("CARGO_BIN_EXE_claude-plus")])
        .arg(&dist));
    assert!(packaged.status.success());
    let assets = run(Command::new("sh")
        .arg(script("release-assets.sh"))
        .args([VERSION, "ada"])
        .arg(&dist));
    assert!(!assets.status.success());
    assert_eq!(
        text(&assets.stderr),
        format!(
            "release-assets.sh: {}/claude-plus-x86_64-apple-darwin.tar.gz is missing\n",
            dist.display()
        )
    );
}

fn workflows() -> Vec<(String, String)> {
    let dir = root().join(".github/workflows");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    found
}

fn is_pinned(action: &str) -> bool {
    let Some((name, rest)) = action.split_once('@') else {
        return false;
    };
    let Some((sha, tag)) = rest.split_once(" # ") else {
        return false;
    };
    name.split('/').count() == 2
        && sha.len() == 40
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && tag.strip_prefix('v').is_some_and(|version| {
            version.split('.').count() == 3 && version.split('.').all(|n| n.parse::<u32>().is_ok())
        })
}

#[test]
fn workflows_pin_every_action_to_a_commit_and_every_runner_to_an_image() {
    let found = workflows();
    assert_eq!(
        found
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["ci.yml", "release.yml"]
    );
    for (name, text) in &found {
        for line in text.lines().map(str::trim) {
            if let Some(action) = line.trim_start_matches("- ").strip_prefix("uses: ") {
                assert!(is_pinned(action), "{name}: {line}");
            }
            if let Some(runner) = line.trim_start_matches("- ").strip_prefix("os: ") {
                for image in runner.trim_matches(['[', ']']).split(", ") {
                    assert!(!image.ends_with("-latest"), "{name}: {line}");
                }
            }
            if let Some(runner) = line.strip_prefix("runs-on: ") {
                assert!(
                    runner == "${{ matrix.os }}"
                        || ["macos-15", "ubuntu-24.04", "ubuntu-24.04-arm"].contains(&runner),
                    "{name}: {line}"
                );
            }
            assert!(!line.contains("pull_request_target"), "{name}: {line}");
            assert!(!line.contains("${{ github.event"), "{name}: {line}");
            assert!(
                !line.contains("| bash") && !line.contains("| sh"),
                "{name}: {line}"
            );
            assert!(
                !line.contains("@stable") && !line.contains("@master"),
                "{name}: {line}"
            );
        }
    }
    assert!(!is_pinned("actions/checkout@v4"));
    assert!(!is_pinned(
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
    ));
    assert!(is_pinned(
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"
    ));
}

#[test]
fn the_release_only_drafts_and_the_tag_must_match_the_version() {
    let (_, release) = workflows()
        .into_iter()
        .find(|(name, _)| name == "release.yml")
        .unwrap();
    assert!(release.contains("on:\n  push:\n    tags: [\"v*\"]\n"));
    assert!(
        release.contains(
            "gh release create \"$GITHUB_REF_NAME\" --repo \"$GITHUB_REPOSITORY\" --draft "
        )
    );
    assert_eq!(release.matches("contents: write").count(), 1);
    assert!(release.contains("  test:\n"), "{release}");
    assert!(release.contains("cargo test --locked"), "{release}");
    assert!(release.contains("    needs: test\n"), "{release}");
    assert!(
        release.contains("mod/.claude-plugin/plugin.json"),
        "{release}"
    );
    assert!(release.contains("arch -x86_64"), "{release}");
    for target in TARGETS {
        assert!(
            release.contains(&format!("          - target: {target}\n")),
            "{target}"
        );
    }
}
