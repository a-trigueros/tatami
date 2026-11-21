//! `tt list` — all workspaces with their working-copy status.

use crate::jj::{Jj, JjError, WorkspaceInfo};

pub fn run(jj: &dyn Jj) -> Result<(), JjError> {
    let workspaces = jj.workspaces()?;
    let rows: Vec<ListRow> = workspaces.iter().map(display_row).collect();
    print!("{}", render(&rows));
    Ok(())
}

/// What [`render`] draws. On-disk presence is resolved once here; rendering
/// itself never touches the filesystem.
struct ListRow {
    name: String,
    current: bool,
    change: String,
    description: String,
    flags: String,
    root: String,
}

fn display_row(workspace: &WorkspaceInfo) -> ListRow {
    let mut flags = Vec::new();
    if workspace.wc.empty {
        flags.push("empty");
    }
    if workspace.wc.conflict {
        flags.push("conflict");
    }
    if workspace.wc.divergent {
        flags.push("divergent");
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!("({})", flags.join(", "))
    };

    let root = match &workspace.root {
        Some(root) if root.exists() => root.display().to_string(),
        Some(root) => format!("{} (missing)", root.display()),
        None => "-".to_string(),
    };

    ListRow {
        name: workspace.name.clone(),
        current: workspace.is_current,
        change: workspace.wc.change.clone(),
        description: if workspace.wc.description.is_empty() {
            "(no description set)".to_string()
        } else {
            workspace.wc.description.clone()
        },
        flags,
        root,
    }
}

/// Column-aligned rows, two spaces apart, no trailing whitespace. No header —
/// the output reads like `jj log`: one line per workspace.
fn render(rows: &[ListRow]) -> String {
    let marker_w = 1;
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let change_w = rows.iter().map(|r| r.change.len()).max().unwrap_or(0);
    let description_w = rows.iter().map(|r| r.description.len()).max().unwrap_or(0);
    let flags_w = rows.iter().map(|r| r.flags.len()).max().unwrap_or(0);

    let mut out = String::new();
    for r in rows {
        let line = format!(
            "{:<mw$} {:<nw$}  {:<cw$}  {:<dw$}  {:<fw$}  {}",
            if r.current { "*" } else { " " },
            r.name,
            r.change,
            r.description,
            r.flags,
            r.root,
            mw = marker_w,
            nw = name_w,
            cw = change_w,
            dw = description_w,
            fw = flags_w,
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod display_row_should {
    use super::*;
    use crate::jj::WcStatus;
    use std::path::PathBuf;

    fn info(root: Option<PathBuf>, description: &str) -> WorkspaceInfo {
        WorkspaceInfo {
            name: "ws".to_string(),
            root,
            is_current: false,
            wc: WcStatus {
                change: "szyooy".to_string(),
                description: description.to_string(),
                empty: false,
                conflict: false,
                divergent: false,
            },
        }
    }

    #[test]
    fn shows_the_jj_no_description_placeholder_for_undescribed_workspaces() {
        let row = display_row(&info(None, ""));
        assert_eq!(row.description, "(no description set)");
    }

    #[test]
    fn keeps_descriptions_as_jj_reports_them() {
        let row = display_row(&info(None, "hello main"));
        assert_eq!(row.description, "hello main");
    }

    #[test]
    fn marks_a_root_missing_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("removed-workspace");

        let row = display_row(&info(Some(gone), "d"));

        assert!(row.root.ends_with(" (missing)"), "root was: {}", row.root);
    }

    #[test]
    fn keeps_a_root_present_on_disk_undecorated() {
        let dir = tempfile::tempdir().unwrap();

        let row = display_row(&info(Some(dir.path().to_path_buf()), "d"));

        assert!(!row.root.contains("(missing)"), "root was: {}", row.root);
        assert_eq!(row.root, dir.path().display().to_string());
    }

    #[test]
    fn shows_a_dash_when_the_root_is_not_recorded() {
        let row = display_row(&info(None, "d"));
        assert_eq!(row.root, "-");
    }

    #[test]
    fn lists_all_status_flags() {
        let mut ws = info(None, "d");
        ws.wc.empty = true;
        ws.wc.conflict = true;
        ws.wc.divergent = true;

        let row = display_row(&ws);

        assert_eq!(row.flags, "(empty, conflict, divergent)");
    }

    #[test]
    fn leaves_ordinary_working_copies_unflagged() {
        let row = display_row(&info(None, "d"));
        assert_eq!(row.flags, "");
    }
}

#[cfg(test)]
mod render_should {
    use super::*;

    fn row(
        name: &str,
        current: bool,
        change: &str,
        description: &str,
        flags: &str,
        root: String,
    ) -> ListRow {
        ListRow {
            name: name.to_string(),
            current,
            change: change.to_string(),
            description: description.to_string(),
            flags: flags.to_string(),
            root,
        }
    }

    #[test]
    fn marks_only_the_current_workspace() {
        let rows = vec![
            row(
                "default",
                true,
                "szyooy",
                "hello main",
                "",
                "/repo/main".to_string(),
            ),
            row(
                "ws2",
                false,
                "prmqpv",
                "(no description set)",
                "",
                "/repo/ws2".to_string(),
            ),
        ];

        let out = render(&rows);

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("* default"),
            "line 1 was: {:?}",
            lines[0]
        );
        assert!(lines[1].starts_with("  ws2"), "line 2 was: {:?}", lines[1]);
    }

    #[test]
    fn aligns_columns_across_rows_and_trims_trailing_whitespace() {
        let rows = vec![
            row(
                "default",
                true,
                "szyooy",
                "hello main",
                "(empty)",
                "/repo/main".to_string(),
            ),
            row(
                "a",
                false,
                "x",
                "(no description set)",
                "",
                "/repo/a".to_string(),
            ),
        ];

        let out = render(&rows);

        for line in out.lines() {
            assert_eq!(line, line.trim_end(), "trailing whitespace in: {:?}", line);
        }
        // Columns are separated by two-space gutters plus per-column padding;
        // multi-word descriptions contain single spaces, so splitting on the
        // gutter and trimming each token survives them.
        let columns_of_second: Vec<&str> = out
            .lines()
            .last()
            .unwrap()
            .split("  ")
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        assert_eq!(
            columns_of_second,
            ["a", "x", "(no description set)", "/repo/a"]
        );
    }

    #[test]
    fn renders_nothing_for_no_rows() {
        assert_eq!(render(&[]), "");
    }
}
