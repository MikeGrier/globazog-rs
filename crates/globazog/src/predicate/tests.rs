// Copyright (c) 2026 Mike Grier

use super::*;
use crate::syntax::CaseSensitivity;

const CS: CaseSensitivity = CaseSensitivity::Sensitive;
const CI: CaseSensitivity = CaseSensitivity::Insensitive;

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

fn meta(name: &[u32]) -> EntryMeta<'_> {
    EntryMeta {
        name,
        depth: 0,
        entry_type: EntryType::File,
        is_reparse: false,
        reparse_tag: 0,
        attributes: 0,
        size: 0,
        btime: 0,
        mtime: 0,
        atime: 0,
        ctime: 0,
    }
}

#[test]
fn name_exact_and_negate() {
    let n = cps("main.rs");
    let m = meta(&n);
    assert!(eval_leaf(&Leaf::name_exact("main.rs", CS), &m));
    assert!(!eval_leaf(&Leaf::name_exact("other.rs", CS), &m));
    let not = Leaf::Name {
        seg: super::literals("main.rs"),
        case: CS,
        negate: true,
    };
    assert!(!eval_leaf(&not, &m));
}

#[test]
fn name_extension_and_contains() {
    let n = cps("archive.tar.gz");
    let m = meta(&n);
    assert!(eval_leaf(&Leaf::name_extension("gz", CS), &m));
    assert!(!eval_leaf(&Leaf::name_extension("tar", CS), &m));
    assert!(eval_leaf(&Leaf::name_contains("tar", CS), &m));
    assert!(!eval_leaf(&Leaf::name_contains("zip", CS), &m));
}

#[test]
fn name_in_set() {
    let n = cps("Makefile");
    let m = meta(&n);
    assert!(eval_leaf(
        &Leaf::name_in_set(&["Makefile", "BUILD"], CS),
        &m
    ));
    assert!(!eval_leaf(&Leaf::name_in_set(&["a", "b"], CS), &m));
}

#[test]
fn name_case_insensitive() {
    let n = cps("readme.md");
    let m = meta(&n);
    assert!(eval_leaf(&Leaf::name_exact("README.MD", CI), &m));
    assert!(!eval_leaf(&Leaf::name_exact("README.MD", CS), &m));
}

#[test]
fn entry_type() {
    let n = cps("dir");
    let m = EntryMeta {
        entry_type: EntryType::Dir,
        ..meta(&n)
    };
    assert!(eval_leaf(
        &Leaf::IsType {
            ty: EntryType::Dir,
            negate: false
        },
        &m
    ));
    assert!(eval_leaf(
        &Leaf::IsType {
            ty: EntryType::File,
            negate: true
        },
        &m
    ));
}

#[test]
fn reparse_bool_and_tag() {
    let n = cps("link");
    let m = EntryMeta {
        is_reparse: true,
        reparse_tag: 0xA000_000C,
        ..meta(&n)
    };
    assert!(eval_leaf(&Leaf::IsReparse { negate: false }, &m));
    assert!(!eval_leaf(&Leaf::IsReparse { negate: true }, &m));
    assert!(eval_leaf(
        &Leaf::ReparseTag {
            tag: 0xA000_000C,
            negate: false
        },
        &m
    ));
}

#[test]
fn attribute_masks() {
    let hidden = 0x2;
    let system = 0x4;
    let n = cps("f");
    let m = EntryMeta {
        attributes: hidden,
        ..meta(&n)
    };
    assert!(eval_leaf(&Leaf::AttrsAllSet(hidden), &m));
    assert!(!eval_leaf(&Leaf::AttrsAllSet(hidden | system), &m));
    assert!(eval_leaf(&Leaf::AttrsAllClear(system), &m));
    assert!(!eval_leaf(&Leaf::AttrsAllClear(hidden), &m));
}

#[test]
fn size_time_depth_comparisons() {
    let n = cps("f");
    let m = EntryMeta {
        size: 1000,
        mtime: 500,
        depth: 3,
        ..meta(&n)
    };
    assert!(eval_leaf(
        &Leaf::Size {
            op: Cmp::Gt,
            value: 999
        },
        &m
    ));
    assert!(!eval_leaf(
        &Leaf::Size {
            op: Cmp::Lt,
            value: 1000
        },
        &m
    ));
    assert!(eval_leaf(
        &Leaf::Time {
            field: TimeField::Mtime,
            op: Cmp::Ge,
            value: 500
        },
        &m
    ));
    assert!(eval_leaf(
        &Leaf::Depth {
            op: Cmp::Eq,
            value: 3
        },
        &m
    ));
}

#[test]
fn conjunction_and_empty() {
    let n = cps("app.log");
    let m = EntryMeta {
        size: 2000,
        ..meta(&n)
    };
    let pass = vec![
        Leaf::name_extension("log", CS),
        Leaf::Size {
            op: Cmp::Gt,
            value: 1000,
        },
    ];
    let fail = vec![
        Leaf::name_extension("log", CS),
        Leaf::Size {
            op: Cmp::Gt,
            value: 5000,
        },
    ];
    assert!(eval_all(&pass, &m));
    assert!(!eval_all(&fail, &m));
    assert!(eval_all(&[], &m)); // empty conjunction is vacuously true
}

#[test]
fn fetch_mask_union() {
    let leaves = vec![
        Leaf::Size {
            op: Cmp::Gt,
            value: 0,
        },
        Leaf::Time {
            field: TimeField::Mtime,
            op: Cmp::Gt,
            value: 0,
        },
        Leaf::IsType {
            ty: EntryType::File,
            negate: false,
        },
    ];
    assert_eq!(
        required_fields(&leaves),
        MetaMask::SIZE | MetaMask::MTIME | MetaMask::TYPE
    );

    // Name and depth conditions need no stat-tier fetch.
    let cheap = vec![
        Leaf::name_exact("x", CS),
        Leaf::Depth {
            op: Cmp::Lt,
            value: 5,
        },
    ];
    assert_eq!(required_fields(&cheap), MetaMask::empty());
}

#[test]
fn d66_per_pattern_emit_example() {
    // emit for `**/*.log`: over 10 MiB; emit for `**/*.conf`: any size (D-66).
    let ten_mib = 10 * 1024 * 1024;
    let emit_log = vec![
        Leaf::name_extension("log", CS),
        Leaf::Size {
            op: Cmp::Gt,
            value: ten_mib,
        },
    ];
    let emit_conf = vec![Leaf::name_extension("conf", CS)];

    let conf_name = cps("app.conf");
    let log_name = cps("app.log");
    let small_conf = EntryMeta {
        size: 2000,
        ..meta(&conf_name)
    };
    let small_log = EntryMeta {
        size: 2000,
        ..meta(&log_name)
    };
    let big_log = EntryMeta {
        size: 20 * 1024 * 1024,
        ..meta(&log_name)
    };

    assert!(eval_all(&emit_conf, &small_conf)); // small .conf is emitted
    assert!(!eval_all(&emit_log, &small_log)); // small .log is NOT emitted
    assert!(eval_all(&emit_log, &big_log)); // big .log is emitted
    assert!(!eval_all(&emit_conf, &small_log)); // .log not matched by the .conf emit
}
