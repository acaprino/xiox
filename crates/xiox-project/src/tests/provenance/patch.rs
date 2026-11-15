use crate::provenance::{Completeness, PatchAction, patch_ops};

use Completeness::{Truncated, Whole};
use PatchAction::{Add, Delete, MoveFrom, MoveTo, Update};

#[test]
fn patch_headers() {
    let cases: [(&str, &str, Completeness, Vec<(PatchAction, &str)>); 7] = [
        ("add", "*** Begin Patch\n*** Add File: a/b.rs\n+fn main() {}\n*** End Patch\n", Whole, vec![(Add, "a/b.rs")]),
        (
            "update and delete",
            "*** Begin Patch\n*** Update File: src/x.rs\n@@\n-a\n+b\n*** Delete File: old.txt\n*** End Patch",
            Whole,
            vec![(Update, "src/x.rs"), (Delete, "old.txt")],
        ),
        (
            "move",
            "*** Update File: a.rs\n*** Move to: b.rs\n@@\n-x\n+y\n",
            Whole,
            vec![(MoveFrom, "a.rs"), (MoveTo, "b.rs")],
        ),
        (
            "content lines do not count",
            "*** Add File: n.md\n+*** Add File: fake\n+ *** Delete File: fake\n-*** Update File: fake\n",
            Whole,
            vec![(Add, "n.md")],
        ),
        ("cut header, truncated", "*** Add File: x\n+a\n*** Add File: D:/a/b", Truncated, vec![(Add, "x")]),
        (
            "cut header, whole",
            "*** Add File: x\n+a\n*** Add File: D:/a/b",
            Whole,
            vec![(Add, "x"), (Add, "D:/a/b")],
        ),
        (
            "crlf and padding",
            "*** Update File:  src/a.rs \r\n@@\r\n*** Delete File: b.rs\r\n",
            Truncated,
            vec![(Update, "src/a.rs"), (Delete, "b.rs")],
        ),
    ];
    for (name, text, completeness, want) in cases {
        let got = patch_ops(text, completeness);
        let want: Vec<(PatchAction, String)> = want.into_iter().map(|(a, p)| (a, p.to_string())).collect();
        assert_eq!(got, want, "{name}");
    }
}
