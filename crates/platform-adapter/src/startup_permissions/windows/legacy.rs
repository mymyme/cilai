//! Recognize one legacy ACL conversion, never general permission equivalence.
//! The inherited-source bit (ID) does not change an ACE's access mask. Windows
//! can remove it from a legacy child while a parent is being made private.
//! This exception is only for a pinned directory that this approved repair is
//! about to replace with a protected private DACL, after repairing its parent.

pub(super) fn inherited_source_removed(before: &str, after: &str) -> bool {
    let Some((before_identity, before_acl)) = before.split_once("D:") else {
        return false;
    };
    let Some((after_identity, after_acl)) = after.split_once("D:") else {
        return false;
    };
    // Exact owner/group and legacy control flags: no P, AI, AR or SACL suffix.
    // Modern/protected ACLs and all unknown representations still conflict.
    if before_identity != after_identity
        || !before_acl.starts_with('(')
        || !after_acl.starts_with('(')
    {
        return false;
    }
    let (Some(before_entries), Some(after_entries)) = (entries(before_acl), entries(after_acl))
    else {
        return false;
    };
    if before_entries.len() != after_entries.len() {
        return false;
    }
    let mut removed = false;
    for (before, after) in before_entries.iter().zip(&after_entries) {
        // ACE order/type, rights, object fields and SID must all stay identical.
        if [0, 2, 3, 4, 5].iter().any(|&i| before[i] != after[i]) {
            return false;
        }
        if before[1] == after[1] {
            continue;
        }
        if !before[1].contains("ID") || before[1].replace("ID", "") != after[1] {
            return false;
        }
        removed = true;
    }
    removed
}

fn entries(mut acl: &str) -> Option<Vec<Vec<&str>>> {
    let mut entries = Vec::new();
    while !acl.is_empty() {
        let rest = acl.strip_prefix('(')?;
        let (entry, tail) = rest.split_once(')')?;
        let fields = entry.split(';').collect::<Vec<_>>();
        if fields.len() != 6
            || !matches!(fields[0], "A" | "D")
            || !fields[3].is_empty()
            || !fields[4].is_empty()
            || fields[2].is_empty()
            || fields[5].is_empty()
            || !fields[1].len().is_multiple_of(2)
        {
            return None;
        }
        let mut seen = Vec::new();
        for flag in fields[1].as_bytes().chunks_exact(2) {
            if !matches!(flag, b"OI" | b"CI" | b"IO" | b"NP" | b"ID") || seen.contains(&flag) {
                return None;
            }
            seen.push(flag);
        }
        entries.push(fields);
        acl = tail;
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEFORE: &str =
        "O:BAG:S-1-5-21-1-2-3-513D:(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIID;FA;;;LA)";

    #[test]
    fn recognizes_the_captured_legacy_inherited_source_conversion() {
        assert!(inherited_source_removed(
            BEFORE,
            &BEFORE.replace("OICIID", "OICI")
        ));
        assert!(inherited_source_removed(
            BEFORE,
            &BEFORE.replacen("OICIID", "OICI", 1)
        ));
        assert!(!inherited_source_removed(BEFORE, BEFORE));
        assert!(!inherited_source_removed(
            &BEFORE.replace("OICIID", "OICI"),
            BEFORE
        ));
    }

    #[test]
    fn rejects_access_identity_order_inheritance_and_descriptor_changes() {
        let converted = BEFORE.replace("OICIID", "OICI");
        for changed in [
            converted.replace("O:BA", "O:LA"),
            converted.replace("-513D:", "-514D:"),
            converted.replacen(";FA;", ";FR;", 1),
            converted.replace(";;;LA)", ";;;WD)"),
            converted.replace("(A;OICI;FA;;;BA)", ""),
            format!("{converted}(A;;FA;;;WD)"),
            converted.replacen("(A;", "(D;", 1),
            converted.replacen("OICI", "CI", 1),
            converted.replacen("OICI", "OICIIO", 1),
            converted.replacen("OICI", "OICINP", 1),
            converted.replace("D:", "D:P"),
            converted.replace("D:", "D:AI"),
            converted.replace("D:", "D:AR"),
            converted.replace(
                "(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)",
                "(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)",
            ),
            format!("{converted}S:(AU;SA;FA;;;WD)"),
        ] {
            assert!(!inherited_source_removed(BEFORE, &changed), "{changed}");
        }
        for flags in ["P", "AI", "AR"] {
            assert!(!inherited_source_removed(
                &BEFORE.replace("D:", &format!("D:{flags}")),
                &converted.replace("D:", &format!("D:{flags}"))
            ));
        }
        for unsupported in [
            "(XA;OICIID;FA;;;SY)",
            "(OA;OICIID;FA;;;SY)",
            "(A;OICIIDID;FA;;;SY)",
            "(A;OICIIDZZ;FA;;;SY)",
        ] {
            let descriptor = format!("O:BAG:SYD:{unsupported}");
            assert!(!inherited_source_removed(
                &descriptor,
                &descriptor.replace("ID", "")
            ));
        }
    }
}
