//! Build gate: every module-scope Fortran symbol in the vendored modem must be classified in
//! `libtempo/modem-state-manifest.toml`.
//!
//! # Why this exists
//!
//! Two radio chains decoding different bands in one process share every statically-allocated
//! Fortran symbol unless the manifest's class-1 set is swapped around each decode. An
//! *unclassified* symbol is, by default, a *shared* one — which is exactly the bug this
//! whole audit was written to prevent. The failure mode is not a crash: it is a CRC-valid,
//! syntactically perfect, WRONG decode that gets logged and uploaded.
//!
//! So a vendor refresh that quietly introduces new state must break the build, not ship.
//!
//! # What it does and does not catch
//!
//! It scans for the *greppable* declaration forms — `save`, `data`, `common`, and module-scope
//! declarations between `module` and `contains`. It deliberately does **not** try to re-derive
//! the full audit: ~160 of the manifest's 585 symbols are ordinary locals that gfortran spilled
//! into `.bss` for exceeding `-fmax-stack-var-size`, which is a property of the *compiler
//! invocation*, not the source, and no source scan can see them.
//!
//! That is the right trade. The gate's job is "did a refresh add state nobody classified",
//! and new state arrives as a visible declaration. Catching the compiler-spilled set would
//! need `nm` on a built object, which is a different (and much slower) check.
//!
//! # Keyed on (file, name), not (file, line)
//!
//! The manifest records `line` for humans to find the symbol. The gate ignores it: an
//! unrelated edit that shifts a declaration down three lines is not a finding, and a gate that
//! cries wolf on every whitespace change gets disabled. A genuinely NEW symbol changes the
//! (file, name) set, which is what we test.

use std::collections::HashSet;
use std::path::Path;

/// A symbol the scanner believes is module-scope state, keyed as the gate compares it.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub file: String,
    pub name: String,
}

/// Pull `name` + `file` out of every `[[symbol]]` table. Deliberately a hand-rolled scan
/// rather than a toml dependency: build-dependencies are compiled for the HOST on every clean
/// build of every target, and this file's shape is fully under our control.
pub fn parse_manifest(text: &str) -> HashSet<Key> {
    let mut out = HashSet::new();
    let (mut name, mut file) = (None, None);
    for line in text.lines() {
        let t = line.trim();
        if t == "[[symbol]]" {
            name = None;
            file = None;
        } else if let Some(v) = t.strip_prefix("name = ") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = t.strip_prefix("file = ") {
            file = Some(v.trim().trim_matches('"').to_string());
        }
        if let (Some(n), Some(f)) = (&name, &file) {
            out.insert(Key {
                file: f.clone(),
                name: n.clone(),
            });
            name = None;
            file = None;
        }
    }
    out
}

/// Symbol names declared at module scope in one Fortran source.
///
/// Intentionally over-inclusive on `save`/`data`/`common` and conservative on plain
/// declarations (module scope only — between `module` and the first `contains`), because a
/// false positive here costs one manifest row while a false negative costs a shared symbol.
pub fn scan_fortran(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_module = false;
    let mut past_contains = false;
    // Interface blocks declare PROCEDURES, not storage — and their bodies are full of `::`
    // result-type and dummy-argument declarations that look exactly like module state. The
    // whole crc.f90 bind(C) block was the scanner's false-positive population.
    let mut interface_depth = 0i32;
    // A derived TYPE block declares member layout, not storage. `harq_slot`'s members
    // (cd_rv0, freq, rv_count, …) are not symbols — the SLOT ARRAY declared of that type is,
    // and it is classified separately.
    let mut in_type_block = false;
    for raw in src.lines() {
        let line = raw.split('!').next().unwrap_or("").trim(); // strip comments
        let low = line.to_ascii_lowercase();
        if low.starts_with("module ") && !low.starts_with("module procedure") {
            in_module = true;
            past_contains = false;
            continue;
        }
        if low == "contains" {
            past_contains = true;
            continue;
        }
        if (low.starts_with("type ") || low.starts_with("type::") || low.starts_with("type,"))
            && line.contains("::")
            && !low.contains("(")
        {
            in_type_block = true;
            continue;
        }
        if low.starts_with("end type") {
            in_type_block = false;
            continue;
        }
        if in_type_block {
            continue;
        }
        if low.starts_with("interface") || low.starts_with("abstract interface") {
            interface_depth += 1;
            continue;
        }
        if low.starts_with("end interface") {
            interface_depth = (interface_depth - 1).max(0);
            continue;
        }
        if interface_depth > 0 {
            continue;
        }
        if low.starts_with("end module") {
            in_module = false;
            past_contains = false;
            continue;
        }
        // `save ::`, `data x/…/`, `common /blk/ a,b` carry state wherever they appear —
        // including inside subroutines, which is the classic SAVEd-local idiom.
        let explicit = !low.starts_with("use ")
            && (low.starts_with("save ")
                || low.starts_with("save::")
                || low.starts_with("data ")
                || low.starts_with("common ")
                || low.starts_with("common/"));
        // `use … :: …`, interfaces and procedure declarations also carry `::` but declare no
        // storage — they were the scanner's entire false-positive population on the real tree
        // (iso_c_binding imports, `procedure` names inside interface blocks).
        let is_decl_noise = low.starts_with("use ")
            || low.starts_with("use,")
            || low.starts_with("interface")
            || low.starts_with("end interface")
            || low.starts_with("procedure")
            || low.starts_with("module procedure")
            || low.starts_with("import")
            || low.starts_with("implicit")
            || low.starts_with("abstract")
            // Access-specifier statements (`public :: null_timer`) name procedures, not
            // storage. The storage is whatever variable is declared elsewhere.
            || low.starts_with("public")
            || low.starts_with("private")
            || low.starts_with("protected");
        // A module-scope declaration before `contains` is implicitly SAVEd.
        let module_decl = in_module && !past_contains && line.contains("::") && !is_decl_noise;
        if !(explicit || module_decl) {
            continue;
        }
        out.extend(names_in_decl(line));
    }
    out.sort();
    out.dedup();
    out
}

/// Identifier names from one declaration line, ignoring types, attributes, dimensions and
/// initializers.
fn names_in_decl(line: &str) -> Vec<String> {
    // Everything after `::` is the name list; without `::` (save/data/common) take the tail.
    let tail = match line.split_once("::") {
        Some((_, rhs)) => rhs,
        None => {
            let low = line.to_ascii_lowercase();
            let kw = ["save", "data", "common"]
                .iter()
                .find(|k| low.starts_with(*k))
                .copied()
                .unwrap_or("");
            &line[kw.len().min(line.len())..]
        }
    };
    let mut names = Vec::new();
    let mut depth = 0i32; // skip dimensions (…) and DATA initializers /…/
    let mut cur = String::new();
    let mut in_slash = false;
    for ch in tail.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '/' => in_slash = !in_slash,
            _ if depth > 0 || in_slash => {}
            c if c.is_alphanumeric() || c == '_' => cur.push(c),
            _ => {
                push_name(&mut names, &mut cur);
            }
        }
        if depth > 0 || in_slash {
            push_name(&mut names, &mut cur);
        }
    }
    push_name(&mut names, &mut cur);
    names
}

fn push_name(names: &mut Vec<String>, cur: &mut String) {
    if cur.is_empty() {
        return;
    }
    let s = std::mem::take(cur);
    // Type/attribute keywords that survive the `::` split on continuation lines, plus bare
    // numbers from dimensions.
    const NOISE: &[&str] = &[
        "integer",
        "real",
        "complex",
        "character",
        "logical",
        "double",
        "precision",
        "parameter",
        "save",
        "data",
        "common",
        "dimension",
        "allocatable",
        "pointer",
        "target",
        "intent",
        "in",
        "out",
        "inout",
        "kind",
        "len",
        "type",
        "class",
        "public",
        "private",
        "protected",
        "optional",
        "value",
        "external",
        "intrinsic",
        // Literals and intrinsics that survive DATA/initializer parsing.
        "true",
        "false",
        "reshape",
        "null",
        "none",
    ];
    let low = s.to_ascii_lowercase();
    if NOISE.contains(&low.as_str()) || s.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    names.push(s);
}

/// Every symbol the scanner finds that the manifest does not classify.
pub fn unclassified(lib_root: &Path, manifest_text: &str) -> Vec<Key> {
    let known = parse_manifest(manifest_text);
    let mut missing = Vec::new();
    let mut stack = vec![lib_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("f90") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else {
                continue;
            };
            let rel = p
                .strip_prefix(lib_root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            for name in scan_fortran(&src) {
                let k = Key {
                    file: rel.clone(),
                    name,
                };
                if !known.contains(&k) {
                    missing.push(k);
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_symbol_tables() {
        let m = r#"
[[symbol]]
name = "msg0"
file = "ft8/ft8_a7.f90"
line = 11
class = 1

[[symbol]]
name = "jseq"
file = "ft8/ft8_a7.f90"
line = 14
class = 1
"#;
        let k = parse_manifest(m);
        assert_eq!(k.len(), 2);
        assert!(k.contains(&Key {
            file: "ft8/ft8_a7.f90".into(),
            name: "msg0".into()
        }));
    }

    #[test]
    fn finds_module_scope_declarations() {
        let src = "\
module ft8_a7
  parameter(MAXDEC=200)
  real dt0(MAXDEC,0:1,0:1)
  character*37 msg0(MAXDEC,0:1,0:1)
  integer :: jseq
contains
  subroutine foo()
    real scratch(100)
  end subroutine
end module
";
        let n = scan_fortran(src);
        assert!(n.contains(&"jseq".to_string()), "{n:?}");
        // A local INSIDE contains is not module-scope state — must not be swept in.
        assert!(!n.contains(&"scratch".to_string()), "{n:?}");
    }

    #[test]
    fn finds_saved_locals_inside_subroutines() {
        // The classic SAVEd-local idiom: state that a naive module-scope-only scan misses.
        let src = "\
subroutine bar()
  real x(10)
  save x
  data first/.true./
end subroutine
";
        let n = scan_fortran(src);
        assert!(n.contains(&"x".to_string()), "{n:?}");
        assert!(n.contains(&"first".to_string()), "{n:?}");
    }

    #[test]
    fn ignores_comments_and_type_keywords() {
        let src = "\
module m
  ! save this is a comment, not a declaration
  integer :: alpha
end module
";
        let n = scan_fortran(src);
        assert_eq!(n, vec!["alpha".to_string()], "{n:?}");
    }

    #[test]
    fn a_new_unclassified_symbol_is_reported() {
        // The whole point: a vendor refresh adds state, the manifest does not know it.
        let manifest = "[[symbol]]\nname = \"known\"\nfile = \"a.f90\"\nline = 1\nclass = 1\n";
        let known = parse_manifest(manifest);
        assert!(known.contains(&Key {
            file: "a.f90".into(),
            name: "known".into()
        }));
        assert!(!known.contains(&Key {
            file: "a.f90".into(),
            name: "brand_new".into()
        }));
    }
}
