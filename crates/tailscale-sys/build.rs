use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let upstream = manifest.join("libtailscale");
    let patches = manifest.join("patches");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", patches.display());
    for file in [
        "tailscale.go",
        "tailscale.c",
        "tailscale.h",
        "go.mod",
        "go.sum",
    ] {
        println!("cargo:rerun-if-changed={}", upstream.join(file).display());
    }
    for var in [
        "MACOSX_DEPLOYMENT_TARGET",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "GO",
        "CC",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    assert!(
        upstream.join("tailscale.go").exists(),
        "libtailscale submodule missing; run `git submodule update --init`"
    );

    let src = out.join("libtailscale");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src).unwrap();
    for entry in std::fs::read_dir(&upstream).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() && !path.to_string_lossy().ends_with("_test.go") {
            std::fs::copy(&path, src.join(path.file_name().unwrap())).unwrap();
        }
    }
    let mut series: Vec<PathBuf> = std::fs::read_dir(&patches)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "patch"))
        .collect();
    series.sort();
    for patch in &series {
        run(Command::new("patch")
            .args(["-p1", "--forward", "--quiet", "-d"])
            .arg(&src)
            .arg("-i")
            .arg(patch));
    }

    let go_bin = env::var("GO").unwrap_or_else(|_| "go".into());
    vendor_with_patches(&go_bin, &src, &patches);

    let go = GoTarget::from_rust(&target);
    let archive = out.join("libtailscale.a");
    let mut cmd = Command::new(&go_bin);
    cmd.current_dir(&src)
        .env("CGO_ENABLED", "1")
        .env("GOOS", go.goos)
        .env("GOARCH", go.goarch)
        .env("GOTOOLCHAIN", "local")
        .env("GOFLAGS", "-mod=vendor")
        .env("CC", go.cc())
        .env("CGO_CFLAGS", go.cflags())
        .env("CGO_LDFLAGS", go.cflags())
        .args([
            "build",
            "-buildmode=c-archive",
            "-trimpath",
            "-buildvcs=false",
            "-ldflags=-w",
        ]);
    if go.goos == "ios" {
        // Every netcheck probes the gateway for NAT-PMP/PCP/UPnP, which carriers never
        // offer. The Mac keeps its port mapper, so a direct path still forms from its side.
        cmd.arg("-tags=ios,ts_omit_portmapper");
    }
    run(cmd.arg("-o").arg(&archive));

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=tailscale");
    if go.apple.is_some() {
        let mut frameworks = vec!["CoreFoundation", "Security"];
        if go.goos == "darwin" {
            frameworks.push("IOKit");
        }
        for fw in frameworks {
            println!("cargo:rustc-link-lib=framework={fw}");
        }
        println!("cargo:rustc-link-lib=resolv");
    }
    println!("cargo:archive={}", archive.display());

    bindgen::Builder::default()
        .header(src.join("tailscale.h").to_str().unwrap())
        .allowlist_function("tailscale_.*")
        .allowlist_type("tailscale.*")
        .generate()
        .expect("bindgen tailscale.h")
        .write_to_file(out.join("bindings.rs"))
        .unwrap();
}

struct GoTarget {
    goos: &'static str,
    goarch: &'static str,
    apple: Option<AppleSdk>,
}

struct AppleSdk {
    sdk: &'static str,
    clang_target: String,
}

impl GoTarget {
    fn from_rust(target: &str) -> Self {
        let macos = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".into());
        let ios = env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "26.0".into());
        let apple = |sdk, clang_target| Some(AppleSdk { sdk, clang_target });
        let (goos, goarch, apple) = match target {
            "aarch64-apple-darwin" => (
                "darwin",
                "arm64",
                apple("macosx", format!("arm64-apple-macos{macos}")),
            ),
            "aarch64-apple-ios" => (
                "ios",
                "arm64",
                apple("iphoneos", format!("arm64-apple-ios{ios}")),
            ),
            "aarch64-apple-ios-sim" => (
                "ios",
                "arm64",
                apple("iphonesimulator", format!("arm64-apple-ios{ios}-simulator")),
            ),
            "x86_64-unknown-linux-gnu" => ("linux", "amd64", None),
            "aarch64-unknown-linux-gnu" => ("linux", "arm64", None),
            other => panic!("tailscale-sys: unsupported target {other}"),
        };
        Self {
            goos,
            goarch,
            apple,
        }
    }

    /// Linux takes `CC` from the environment (a cross compiler for a foreign target).
    fn cc(&self) -> String {
        match &self.apple {
            Some(a) => xcrun(a.sdk, &["-f", "clang"]),
            None => env::var("CC").unwrap_or_else(|_| "cc".into()),
        }
    }

    fn cflags(&self) -> String {
        match &self.apple {
            Some(a) => format!(
                "-target {} -isysroot {}",
                a.clang_target,
                xcrun(a.sdk, &["--show-sdk-path"])
            ),
            None => String::new(),
        }
    }
}

/// Vendors the module graph (go.sum still verifies every module) so that files of
/// dependencies can be patched; GOMODCACHE files cannot be overlaid.
/// `patches/<import path>/<file>.patch` patches `vendor/<import path>/<file>`.
fn vendor_with_patches(go: &str, src: &Path, patch_root: &Path) {
    run(Command::new(go)
        .current_dir(src)
        .env("GOTOOLCHAIN", "local")
        .env("GOFLAGS", "-mod=mod")
        .args(["mod", "vendor"]));
    let vendor = src.join("vendor");
    let mut stack: Vec<PathBuf> = std::fs::read_dir(patch_root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(patch_root).unwrap().with_extension("");
            run(Command::new("patch")
                .args(["--forward", "--quiet"])
                .arg(vendor.join(&rel))
                .arg("-i")
                .arg(&path));
        }
    }
}

fn xcrun(sdk: &str, args: &[&str]) -> String {
    let out = Command::new("xcrun")
        .args(["--sdk", sdk])
        .args(args)
        .output()
        .expect("xcrun");
    assert!(out.status.success(), "xcrun --sdk {sdk} {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn run(cmd: &mut Command) {
    let status = cmd.status().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed with {status}");
}
