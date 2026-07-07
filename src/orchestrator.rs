use std::collections::BTreeSet;

pub const HIDDEN_PANE_BASE: usize = 10_000;
pub const MAX_GROUPS: usize = PALETTE.len();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupColor {
    pub name: &'static str,
    pub rgb: (u8, u8, u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendBlock {
    pub targets: SendTargets,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendTargets {
    All,
    Panes(BTreeSet<usize>),
}

const PALETTE: [GroupColor; 6] = [
    GroupColor {
        name: "blue",
        rgb: (82, 166, 255),
    },
    GroupColor {
        name: "violet",
        rgb: (176, 132, 255),
    },
    GroupColor {
        name: "rose",
        rgb: (255, 111, 145),
    },
    GroupColor {
        name: "amber",
        rgb: (245, 173, 66),
    },
    GroupColor {
        name: "magenta",
        rgb: (236, 101, 255),
    },
    GroupColor {
        name: "slate",
        rgb: (136, 160, 185),
    },
];

pub fn group_color(index: usize) -> GroupColor {
    PALETTE[index % PALETTE.len()]
}

pub fn group_label(index: usize) -> char {
    (b'A' + (index % 26) as u8) as char
}

pub fn manager_pane_id(group_id: GroupId) -> usize {
    HIDDEN_PANE_BASE + group_id.0
}

pub fn extract_send_blocks(buffer: &mut String) -> Vec<SendBlock> {
    let mut blocks = Vec::new();

    loop {
        let Some(start) = buffer.find("```gridbash send") else {
            trim_unmatched_buffer(buffer);
            break;
        };

        if start > 0 {
            buffer.drain(..start);
        }

        let Some(header_end) = buffer.find('\n') else {
            break;
        };
        let content_start = header_end + 1;
        let Some(close_offset) = buffer[content_start..].find("\n```") else {
            break;
        };
        let content_end = content_start + close_offset;

        let header = buffer[..header_end].trim();
        let body = buffer[content_start..content_end].trim();
        if let Some(targets) = parse_targets(header) {
            if !body.is_empty() {
                blocks.push(SendBlock {
                    targets,
                    message: body.to_string(),
                });
            }
        }

        let close_end = content_end + "\n```".len();
        buffer.drain(..close_end);
    }

    blocks
}

fn trim_unmatched_buffer(buffer: &mut String) {
    const MAX_BUFFER: usize = 4096;
    if buffer.len() > MAX_BUFFER {
        let keep_from = buffer.len().saturating_sub(MAX_BUFFER);
        buffer.drain(..keep_from);
    }
}

fn parse_targets(header: &str) -> Option<SendTargets> {
    let rest = header.strip_prefix("```gridbash send")?.trim();
    if rest.is_empty() || matches!(rest, "all" | "workers") {
        return Some(SendTargets::All);
    }

    let rest = rest
        .strip_prefix("panes")
        .or_else(|| rest.strip_prefix("pane"))
        .unwrap_or(rest)
        .trim();
    let panes = rest
        .split([',', ' ', '\t'])
        .filter(|part| !part.is_empty())
        .map(str::parse::<usize>)
        .collect::<Result<BTreeSet<_>, _>>()
        .ok()?;

    (!panes.is_empty()).then_some(SendTargets::Panes(panes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_deterministic_and_not_green() {
        let names = (0..MAX_GROUPS)
            .map(|index| group_color(index).name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["blue", "violet", "rose", "amber", "magenta", "slate"]
        );
        assert!(!names.contains(&"green"));
    }

    #[test]
    fn extracts_completed_send_blocks() {
        let mut raw = "thinking
```gridbash send panes 1, 3
ship the feature
```
done"
            .to_string();

        let blocks = extract_send_blocks(&mut raw);

        assert_eq!(
            blocks,
            vec![SendBlock {
                targets: SendTargets::Panes(BTreeSet::from([1, 3])),
                message: "ship the feature".into(),
            }]
        );
        assert_eq!(raw, "\ndone");
    }

    #[test]
    fn leaves_incomplete_blocks_buffered() {
        let mut raw = "```gridbash send all\nwait".to_string();

        assert!(extract_send_blocks(&mut raw).is_empty());
        assert_eq!(raw, "```gridbash send all\nwait");
    }
}
