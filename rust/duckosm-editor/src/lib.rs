//! The files of the duckOSM fixes editor (`web/fixes_editor/` of this repository), in the program.
//!
//! A host page serves these files from one folder and runs `import { initFixesPanel } from "./fixes_editor/fixes_panel.js"`.
//! The files are in the library, so a host has no copy of them and no fetch step: it has the editor of the duckOSM version that
//! it depends on, like the engine (docs/design/editor_crate.md).

/// A file of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct File {
    /// The name in the folder `fixes_editor/` (no path).
    pub name: &'static str,
    /// The media type to send with the file.
    pub content_type: &'static str,
    /// The text of the file.
    pub text: &'static str,
}

/// All the files of the editor. `fixes_panel.js` imports `fixes.js` and loads `fixes_panel.css` and `presets.json` from its own folder.
pub const FILES: &[File] = &[
    File { name: "fixes_panel.js", content_type: "text/javascript", text: include_str!("../../../web/fixes_editor/fixes_panel.js") },
    File { name: "fixes.js", content_type: "text/javascript", text: include_str!("../../../web/fixes_editor/fixes.js") },
    File { name: "fixes_panel.css", content_type: "text/css", text: include_str!("../../../web/fixes_editor/fixes_panel.css") },
    File { name: "presets.json", content_type: "application/json", text: include_str!("../../../web/fixes_editor/presets.json") },
];

/// The file with this name, or `None` (a name with a path, such as `../Cargo.toml`, is not a file of the editor).
pub fn file(name: &str) -> Option<&'static File> {
    FILES.iter().find(|f| f.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_files_are_in_the_library() {
        for n in ["fixes_panel.js", "fixes.js", "fixes_panel.css", "presets.json"] {
            assert!(file(n).is_some_and(|f| !f.text.is_empty()), "{n}");
        }
        assert_eq!(FILES.len(), 4);
    }

    #[test]
    fn only_the_files_of_the_editor() {
        for n in ["README.md", "../Cargo.toml", "/etc/passwd", "fixes_units.mjs", ""] {
            assert!(file(n).is_none(), "{n}");
        }
    }

    #[test]
    fn the_files_are_the_files_of_the_folder() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/fixes_editor");
        for f in FILES {
            assert_eq!(std::fs::read_to_string(dir.join(f.name)).unwrap(), f.text, "{}", f.name);
        }
    }

    #[test]
    fn presets_are_json_and_the_panel_imports_what_the_library_has() {
        assert!(file("presets.json").unwrap().text.trim_start().starts_with(['{', '[']));
        // every relative import of the panel is a file of the library
        for line in file("fixes_panel.js").unwrap().text.lines().filter(|l| l.starts_with("import ")) {
            if let Some(p) = line.split('"').nth(1).and_then(|s| s.strip_prefix("./")) {
                assert!(file(p).is_some(), "import {p}");
            }
        }
    }
}
