//! Build `libtempo` (Fortran 4-CPM turbo modem + C ABI) via its CMake project and
//! emit the link directives Cargo needs.
//!
//! The CMake project at `tempo/libtempo` compiles the FT1 modem sources from the
//! WSJT-X FT1 fork into a static `libtempo.a` (target `tempo_static`, OUTPUT_NAME
//! `ft1`). It pulls in the gfortran runtime, FFTW3 single precision, and (for the
//! Boost header-only CRC-14 in `crc14.cpp`) libstdc++.
//!
//! # Cross-compiling Linux host -> Windows (x86_64-pc-windows-gnu)
//! When the Cargo *target* is `x86_64-pc-windows-gnu` but the *host* is not
//! Windows (e.g. building from Linux/WSL with the MinGW-w64 toolchain), the
//! native code path below would be wrong: `cfg!(windows)` reflects the HOST, and
//! CMake would otherwise pick up the host's compilers + host FFTW. The cross
//! path drives CMake with `libtempo/mingw-w64.cmake` (the MinGW-w64 toolchain file)
//! and a statically cross-built FFTW3f, and links everything statically so the
//! resulting Windows binary has no MinGW runtime-DLL dependency.
//!
//! The native (host == target) path is intentionally left byte-for-byte
//! unchanged; all cross logic is gated behind `is_cross_to_windows_gnu()`.

use std::env;
use std::path::PathBuf;
use std::process::Command;

#[path = "manifest_gate.rs"]
mod manifest_gate;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libtempo_src = manifest
        .join("../../libtempo")
        .canonicalize()
        .expect("locate tempo/libtempo");
    check_state_manifest(&libtempo_src);
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Cross-compile branch: Linux (or any non-Windows) host -> Windows GNU target.
    // Everything Windows-cross-specific lives here so the native path is untouched.
    if is_cross_to_windows_gnu() {
        build_cross_windows(&libtempo_src, &out);
        emit_rerun(&libtempo_src);
        return;
    }

    // ---- Native build (host == target): UNCHANGED from the original. ----------
    // Configure.
    let mut cfg = Command::new("cmake");
    cfg.args([
        "-S",
        libtempo_src.to_str().unwrap(),
        "-B",
        out.to_str().unwrap(),
        "-DCMAKE_BUILD_TYPE=Release",
    ]);
    if let Some(wx) = wx_override() {
        cfg.arg(format!("-DWX={wx}"));
    }
    // On Windows the Fortran sources require the GNU toolchain (MSVC has no
    // Fortran). Use Ninja if present, else MinGW Makefiles — both drive gfortran.
    // Build from an MSYS2/MinGW environment with the GNU Rust target
    // (x86_64-pc-windows-gnu). See WINDOWS.md.
    if cfg!(windows) {
        if which("ninja") {
            cfg.args(["-G", "Ninja"]);
        } else {
            cfg.args(["-G", "MinGW Makefiles"]);
        }
    }
    run(&mut cfg);

    // Build just the static archive (skip shared lib + test executables).
    run(Command::new("cmake").args([
        "--build",
        out.to_str().unwrap(),
        "--target",
        "tempo_static",
        "--parallel",
    ]));

    // Link: static libtempo first, then its runtime dependencies.
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=tempo");
    println!("cargo:rustc-link-lib=dylib=gfortran");
    println!("cargo:rustc-link-lib=dylib=fftw3f");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=m");
    // The MinGW gfortran runtime also needs quadmath.
    if cfg!(windows) {
        println!("cargo:rustc-link-lib=dylib=quadmath");
    }

    emit_rerun(&libtempo_src);
}

/// True when cross-compiling to `*-pc-windows-gnu` from a non-Windows host.
/// Native Windows (host == Windows) and native Linux builds both return false.
fn is_cross_to_windows_gnu() -> bool {
    // `cfg!(windows)` is the HOST in a build script. If the host is already
    // Windows this is the native path, not a cross compile.
    if cfg!(windows) {
        return false;
    }
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    target != host
        && env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
}

/// Drive the CMake cross build (MinGW-w64 toolchain + static FFTW3f) and emit
/// static link directives for the gfortran/quadmath/stdc++/fftw3f runtimes so
/// the linked Windows binary needs no MinGW runtime DLLs.
fn build_cross_windows(libtempo_src: &std::path::Path, out: &std::path::Path) {
    let toolchain = libtempo_src.join("mingw-w64.cmake");
    assert!(
        toolchain.exists(),
        "missing MinGW toolchain file: {}",
        toolchain.display()
    );

    // Statically cross-built FFTW3f (single precision). Overridable so a packager
    // can point at a different prefix; defaults to the documented /tmp location.
    let fftw_prefix =
        env::var("FFTW_MINGW_PREFIX").unwrap_or_else(|_| "/tmp/fftw-mingw".to_string());
    assert!(
        PathBuf::from(&fftw_prefix).join("lib/libfftw3f.a").exists(),
        "cross build needs MinGW FFTW3f at {fftw_prefix}/lib/libfftw3f.a — build it with \
         ./configure --host=x86_64-w64-mingw32 --enable-float --enable-static \
         --disable-shared --prefix={fftw_prefix}",
    );

    // Configure with the toolchain file. Prefer Ninja (matches the native path),
    // falling back to Unix Makefiles (the cross compilers are unsuffixed MinGW
    // binaries, so a plain make generator drives them fine).
    let mut cfg = Command::new("cmake");
    cfg.args([
        "-S",
        libtempo_src.to_str().unwrap(),
        "-B",
        out.to_str().unwrap(),
        "-DCMAKE_BUILD_TYPE=Release",
        &format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain.display()),
        &format!("-DFFTW_MINGW_PREFIX={fftw_prefix}"),
    ]);
    if let Some(wx) = wx_override() {
        cfg.arg(format!("-DWX={wx}"));
    }
    if which("ninja") {
        cfg.args(["-G", "Ninja"]);
    } else {
        cfg.args(["-G", "Unix Makefiles"]);
    }
    run(&mut cfg);

    run(Command::new("cmake").args([
        "--build",
        out.to_str().unwrap(),
        "--target",
        "tempo_static",
        "--parallel",
    ]));

    // Link the static libtempo first, then its runtime dependencies — all STATIC so
    // the Windows binary is self-contained (no libgfortran-5/libstdc++-6 DLLs).
    // The gfortran/quadmath/stdc++ static archives live in the MinGW gcc lib dir;
    // add it to the search path so rustc's gcc linker driver finds them.
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={fftw_prefix}/lib");
    if let Some(gcc_lib) = mingw_gcc_lib_dir() {
        println!("cargo:rustc-link-search=native={gcc_lib}");
    }
    println!("cargo:rustc-link-lib=static=tempo");
    // Order matters: ft1 -> fortran runtime -> quadmath, then C++ runtime, fftw.
    println!("cargo:rustc-link-lib=static=gfortran");
    println!("cargo:rustc-link-lib=static=quadmath");
    println!("cargo:rustc-link-lib=static=stdc++");
    println!("cargo:rustc-link-lib=static=fftw3f");
    // m is part of the MinGW CRT (libmsvcrt); no separate libm. stdc++ pulls in
    // what crc14.cpp needs. gcc/pthread come from the gcc driver automatically.
}

/// Locate the MinGW-w64 gcc library directory that holds the static gfortran /
/// quadmath / stdc++ archives, by asking the cross gcc for its libgcc path.
fn mingw_gcc_lib_dir() -> Option<String> {
    let out = Command::new("x86_64-w64-mingw32-gcc")
        .arg("-print-libgcc-file-name")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let p = PathBuf::from(path.trim());
    p.parent().map(|d| d.display().to_string())
}

fn emit_rerun(libtempo_src: &std::path::Path) {
    println!("cargo:rerun-if-changed=build.rs");

    // Tempo-side libtempo sources (the C-ABI shims, DX1, headers, build config).
    // Every *_cabi.f90 MUST be listed: a content edit to one that is NOT watched
    // silently links a STALE libtempo.a (Cargo never re-runs CMake).
    for rel in [
        "CMakeLists.txt",
        "tempofast_cabi.f90",
        "ft8_cabi.f90",
        "ft8_stdcall.f90",
        "ft4_cabi.f90",
        "mingw-w64.cmake",
        "include",
        "dx1",
    ] {
        emit_rerun_glob(&libtempo_src.join(rel));
    }

    // The modem Fortran lives under WX — by default the in-tree vendored copy at
    // libtempo/vendor/wsjtx, or an external WSJT-X checkout when WX is overridden.
    // CMake's own dependency tracking is correct, but Cargo will not re-run CMake
    // (and so links a stale libtempo.a) unless one of these files is registered here.
    // Watch every modem .f90 in the FT1 + FT8 lib dirs per-file: a bare directory
    // rerun-if-changed catches only add/remove, NOT content edits.
    //
    // This MUST resolve the same WX the CMake configure above used, or edits go
    // unnoticed and the stale archive links silently — `emit_rerun_glob` skips
    // missing paths, so a wrong path fails quietly rather than erroring.
    let wx_lib = match wx_override() {
        Some(wx) => PathBuf::from(wx).join("lib"),
        None => libtempo_src.join("vendor/wsjtx/lib"),
    };
    emit_rerun_glob(&wx_lib.join("tempofast")); // TempoFast modem (turbo, ldpc348, bcjr, sync, harq, ...)
    emit_rerun_glob(&wx_lib.join("ft8")); // shared deps (osd174_91, ldpc_174_91 parity, ...)
    emit_rerun_glob(&wx_lib.join("tempofast_decode.f90")); // live decode + HARQ driver
}

/// The `WX` override (WSJT-X-derived modem source tree) if the environment sets one.
///
/// `libtempo/CMakeLists.txt` declares WX as a CACHE PATH defaulting to the in-tree
/// `libtempo/vendor/wsjtx`, so the common case needs no override at all. Setting `WX`
/// points the build at a different checkout (the FT1 research fork, or a container's
/// staged copy) without editing any tracked file.
fn wx_override() -> Option<String> {
    println!("cargo:rerun-if-env-changed=WX");
    env::var("WX").ok().filter(|s| !s.is_empty())
}

/// Emit `cargo:rerun-if-changed` for `p`. If `p` is a directory, recurse one level
/// and emit each contained `.f90`/`.f`/`.cpp`/`.h`/`.txt`/`.cmake` file individually
/// (so content edits — not just add/remove — trigger a rebuild). Missing paths are
/// skipped (emitting a non-existent path would force an unconditional rebuild).
fn emit_rerun_glob(p: &std::path::Path) {
    let Ok(canon) = p.canonicalize() else { return };
    if canon.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&canon) {
            for entry in rd.flatten() {
                let c = entry.path();
                let watch = c
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| matches!(e, "f90" | "f" | "F90" | "cpp" | "c" | "h" | "txt" | "cmake"))
                    .unwrap_or(false);
                if watch {
                    println!("cargo:rerun-if-changed={}", c.display());
                }
            }
        }
    } else {
        println!("cargo:rerun-if-changed={}", canon.display());
    }
}

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed: {cmd:?}");
}

/// True if `bin` is on PATH (uses `where` on Windows, `which` elsewhere).
fn which(bin: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(bin)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Warn on any module-scope Fortran symbol the state manifest does not classify.
///
/// HARD FAILURE. On its first run (2026-07-21) this reported 29 genuine omissions —
/// subtractft8's camp/cfilt/cref/cw, four2a's FFTW planning state, and twkfreq1.f90 which no
/// audit group's file list even enumerated. Those are now classified and the gate is clean, so
/// it fails the build rather than warning: an unclassified symbol is a SHARED one, and the
/// per-chain decoder context must never be built while any remain.
///
/// If this fires after a vendor refresh, classify the symbol in the manifest. Do NOT silence
/// it by deleting rows or loosening the scanner.
fn check_state_manifest(libtempo_src: &std::path::Path) {
    let manifest_path = libtempo_src.join("modem-state-manifest.toml");
    let lib_root = match wx_override() {
        Some(w) => PathBuf::from(w).join("lib"),
        None => libtempo_src.join("vendor/wsjtx/lib"),
    };
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        println!(
            "cargo:warning=state manifest missing at {}",
            manifest_path.display()
        );
        return;
    };
    let missing = manifest_gate::unclassified(&lib_root, &text);
    if missing.is_empty() {
        return;
    }
    let list = missing
        .iter()
        .map(|k| format!("    {} :: {}", k.file, k.name))
        .collect::<Vec<_>>()
        .join("\n");
    panic!(
        "{} Fortran symbol(s) are NOT classified in libtempo/modem-state-manifest.toml:\n{}\n\n\
         An unclassified symbol is a SHARED one. With two radio chains in one process that \
         produces CRC-valid, well-formed, WRONG decodes that get logged and uploaded — not a \
         crash. Classify each symbol in the manifest (class 1 if the evidence is not decisive; \
         a few bytes of memcpy is cheaper than a fabricated QSO). Do not silence this by \
         deleting rows or loosening the scanner.",
        missing.len(),
        list
    );
}
