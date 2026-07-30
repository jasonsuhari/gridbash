use ratatui::layout::Rect;

/// Upper bound on panes in a single grid. Every path that turns a number into
/// per-pane allocations clamps to this, so an out-of-range grid size can never
/// become an allocation the process cannot satisfy.
pub const MAX_PANES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub rows: usize,
    pub columns: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridLayout {
    size: GridSize,
    row_weights: Vec<u16>,
    column_weights: Vec<u16>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Row(usize),
    Column(usize),
}

impl GridSize {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        let (rows, columns) = normalized
            .split_once('x')
            .or_else(|| normalized.split_once(','))
            .or_else(|| normalized.split_once(' '))?;
        let rows = rows.trim().parse().ok()?;
        let columns = columns.trim().parse().ok()?;
        Self::new(rows, columns)
    }

    pub fn from_count(count: usize) -> Self {
        let count = count.clamp(1, MAX_PANES);
        let columns = ((count as f64).sqrt().ceil() as usize).clamp(1, MAX_PANES);
        let rows = count.div_ceil(columns);
        Self { rows, columns }
    }

    pub fn new(rows: usize, columns: usize) -> Option<Self> {
        // `rows * columns` would overflow for values parsed straight out of a
        // `--grid` argument, and a wrapped product can slip past the cap and
        // reach `vec![1000; rows]`.
        if rows == 0 || columns == 0 || rows.checked_mul(columns)? > MAX_PANES {
            return None;
        }
        Some(Self { rows, columns })
    }

    pub fn count(self) -> usize {
        self.rows.saturating_mul(self.columns)
    }

    pub fn compact_for_count(self, count: usize) -> Self {
        // A caller-built `GridSize` can hold zero rows or columns, and
        // `clamp(1, 0)` panics.
        let count = count.clamp(1, self.count().max(1));
        let mut compact = self;

        while compact.columns > 1 && compact.rows * (compact.columns - 1) >= count {
            compact.columns -= 1;
        }
        while compact.rows > 1 && (compact.rows - 1) * compact.columns >= count {
            compact.rows -= 1;
        }

        compact
    }
}

impl GridLayout {
    pub fn new(size: GridSize) -> Self {
        Self {
            size,
            row_weights: vec![1000; size.rows.min(MAX_PANES)],
            column_weights: vec![1000; size.columns.min(MAX_PANES)],
        }
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn set_size(&mut self, size: GridSize) {
        self.size = size;
        resize_weights(&mut self.row_weights, size.rows.min(MAX_PANES));
        resize_weights(&mut self.column_weights, size.columns.min(MAX_PANES));
    }

    pub fn rects(&self, area: Rect, count: usize) -> Vec<Rect> {
        weighted_grid_rects(
            area,
            self.size,
            &self.row_weights,
            &self.column_weights,
            count,
        )
    }

    #[allow(dead_code)]
    pub fn divider_at(&self, area: Rect, x: u16, y: u16) -> Option<Divider> {
        let column_widths = weighted_lengths(area.width, &self.column_weights);
        let row_heights = weighted_lengths(area.height, &self.row_weights);

        let mut cursor_x = area.x;
        for (index, width) in column_widths
            .iter()
            .copied()
            .enumerate()
            .take(column_widths.len().saturating_sub(1))
        {
            cursor_x = cursor_x.saturating_add(width);
            if x.abs_diff(cursor_x) <= 1 && y >= area.y && y < area.y.saturating_add(area.height) {
                return Some(Divider::Column(index));
            }
        }

        let mut cursor_y = area.y;
        for (index, height) in row_heights
            .iter()
            .copied()
            .enumerate()
            .take(row_heights.len().saturating_sub(1))
        {
            cursor_y = cursor_y.saturating_add(height);
            if y.abs_diff(cursor_y) <= 1 && x >= area.x && x < area.x.saturating_add(area.width) {
                return Some(Divider::Row(index));
            }
        }

        None
    }

    #[allow(dead_code)]
    pub fn drag_divider(&mut self, divider: Divider, area: Rect, x: u16, y: u16) {
        match divider {
            Divider::Column(index) => drag_pair(
                &mut self.column_weights,
                index,
                area.width,
                x.saturating_sub(area.x),
            ),
            Divider::Row(index) => drag_pair(
                &mut self.row_weights,
                index,
                area.height,
                y.saturating_sub(area.y),
            ),
        }
    }
}

#[cfg(test)]
fn grid_rects(area: Rect, grid: GridSize, count: usize) -> Vec<Rect> {
    GridLayout::new(grid).rects(area, count)
}

fn weighted_grid_rects(
    area: Rect,
    grid: GridSize,
    row_weights: &[u16],
    column_weights: &[u16],
    count: usize,
) -> Vec<Rect> {
    // `count` reaches here from pane bookkeeping; honouring it verbatim would
    // turn a bad number into an allocation failure rather than an empty grid.
    // `GridSize::new` already caps a valid grid at `MAX_PANES`, so this only
    // bites a size a caller built by hand.
    let count = count.min(MAX_PANES);
    let mut rects = Vec::with_capacity(count);
    if grid.rows == 0 || grid.columns == 0 {
        return rects;
    }

    let column_widths = weighted_lengths(area.width, column_weights);
    let row_heights = weighted_lengths(area.height, row_weights);

    let row_count = row_heights.len();
    let column_count = column_widths.len();
    let mut y = area.y;
    for (row, row_height) in row_heights.into_iter().enumerate() {
        let mut x = area.x;
        for (column, column_width) in column_widths.iter().copied().enumerate() {
            if rects.len() >= count {
                break;
            }

            // Neighbours share the line that divides them: a pane reaches one
            // cell into the next so both draw the same border row or column, and
            // ratatui's border merging collapses the overlap into a single line.
            // Two abutting frames would otherwise spend two cells on what reads
            // as one divider, and the pair of parallel lines makes a grid look
            // like a spreadsheet instead of a workspace. The last row and column
            // have nothing to reach into, so they stay inside `area`.
            rects.push(Rect {
                x,
                y,
                width: column_width.saturating_add(u16::from(column + 1 < column_count)),
                height: row_height.saturating_add(u16::from(row + 1 < row_count)),
            });
            x = x.saturating_add(column_width);
        }
        y = y.saturating_add(row_height);
    }

    rects
}

fn weighted_lengths(total: u16, weights: &[u16]) -> Vec<u16> {
    if weights.is_empty() {
        return Vec::new();
    }

    let weight_total = weights
        .iter()
        .map(|weight| *weight as u32)
        .sum::<u32>()
        .max(1);
    let mut lengths = Vec::with_capacity(weights.len());
    let mut used = 0_u16;

    for (index, weight) in weights.iter().enumerate() {
        let remaining_slots = weights.len() - index;
        let length = if remaining_slots == 1 {
            total.saturating_sub(used)
        } else {
            let raw = ((*weight as u32 * total as u32) / weight_total) as u16;
            raw.max(1).min(
                total
                    .saturating_sub(used)
                    .saturating_sub((remaining_slots - 1) as u16),
            )
        };

        lengths.push(length);
        used = used.saturating_add(length);
    }

    lengths
}

fn resize_weights(weights: &mut Vec<u16>, len: usize) {
    weights.resize(len, 1000);
}

#[allow(dead_code)]
fn drag_pair(weights: &mut [u16], index: usize, total_pixels: u16, target_pixels: u16) {
    if index + 1 >= weights.len() || total_pixels == 0 {
        return;
    }

    let lengths = weighted_lengths(total_pixels, weights);
    let before_pixels = lengths.iter().take(index).copied().sum::<u16>();
    let pair_pixels = lengths[index].saturating_add(lengths[index + 1]).max(2);
    let local_target = target_pixels
        .saturating_sub(before_pixels)
        .clamp(1, pair_pixels.saturating_sub(1));
    let pair_weight = weights[index].saturating_add(weights[index + 1]).max(2);
    let left_weight = ((local_target as u32 * pair_weight as u32) / pair_pixels as u32) as u16;
    weights[index] = left_weight.max(100);
    weights[index + 1] = pair_weight.saturating_sub(weights[index]).max(100);
}

#[allow(dead_code)]
pub fn pane_at(rects: &[Rect], x: u16, y: u16) -> Option<usize> {
    rects.iter().position(|rect| {
        x >= rect.x
            && x < rect.x.saturating_add(rect.width)
            && y >= rect.y
            && y < rect.y.saturating_add(rect.height)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grid() {
        assert_eq!(
            GridSize::parse("2x3"),
            Some(GridSize {
                rows: 2,
                columns: 3
            })
        );
        assert_eq!(GridSize::parse("0x3"), None);
    }

    #[test]
    fn auto_grid_caps_at_hundred() {
        let grid = GridSize::from_count(100);
        assert_eq!(grid.count(), 100);
    }

    /// `rows * columns` overflows for numbers a `--grid` argument can carry, and
    /// a wrapped product used to slip past the pane cap and reach an allocation
    /// the process cannot satisfy.
    #[test]
    fn grid_dimensions_that_overflow_are_rejected() {
        assert_eq!(GridSize::new(usize::MAX, usize::MAX), None);
        assert_eq!(GridSize::new(usize::MAX, 2), None);
        assert_eq!(GridSize::parse("18446744073709551615x2"), None);
        let huge = (1_usize << 40).to_string();
        assert_eq!(GridSize::parse(&format!("{huge}x{huge}")), None);
    }

    /// A caller-built `GridSize` can hold zero rows, and the old
    /// `clamp(1, self.count())` panicked on the crossed bounds.
    #[test]
    fn compacting_an_empty_grid_does_not_panic() {
        let empty = GridSize {
            rows: 0,
            columns: 0,
        };

        assert_eq!(empty.count(), 0);
        assert_eq!(empty.compact_for_count(4), empty);
        assert_eq!(empty.compact_for_count(0), empty);
    }

    /// An out-of-range `GridSize` must not turn into a huge per-row allocation.
    #[test]
    fn layout_weights_stay_bounded_for_an_absurd_grid() {
        let absurd = GridSize {
            rows: usize::MAX,
            columns: usize::MAX,
        };

        let mut layout = GridLayout::new(absurd);
        assert!(layout.rects(Rect::new(0, 0, 40, 10), 4).len() <= 4);
        layout.set_size(absurd);
        assert!(layout.rects(Rect::new(0, 0, 40, 10), usize::MAX).len() <= MAX_PANES);
    }

    #[test]
    fn compacting_grid_removes_columns_before_rows() {
        let grid = GridSize {
            rows: 3,
            columns: 3,
        };

        assert_eq!(
            grid.compact_for_count(6),
            GridSize {
                rows: 3,
                columns: 2,
            }
        );
    }

    #[test]
    fn compacting_two_by_three_for_four_panes_makes_two_by_two() {
        let grid = GridSize {
            rows: 2,
            columns: 3,
        };

        assert_eq!(
            grid.compact_for_count(4),
            GridSize {
                rows: 2,
                columns: 2,
            }
        );
    }

    #[test]
    fn compacting_grid_keeps_enough_cells_for_every_pane() {
        let grid = GridSize {
            rows: 2,
            columns: 3,
        };

        assert_eq!(
            grid.compact_for_count(3),
            GridSize {
                rows: 2,
                columns: 2,
            }
        );
        assert_eq!(
            grid.compact_for_count(1),
            GridSize {
                rows: 1,
                columns: 1,
            }
        );
    }

    #[test]
    fn grid_rects_cover_area() {
        let area = Rect::new(0, 0, 100, 40);
        let rects = grid_rects(
            area,
            GridSize {
                rows: 2,
                columns: 3,
            },
            6,
        );

        assert_eq!(rects.len(), 6);
        // The grid fills the area exactly: the first pane starts at its origin
        // and the last pane in each direction ends on its far edge.
        assert_eq!(rects[0].x, area.x);
        assert_eq!(rects[0].y, area.y);
        assert_eq!(rects[2].right(), area.right());
        assert_eq!(rects[3].bottom(), area.bottom());
    }

    /// Adjacent panes are meant to land on the same divider cell so the two
    /// borders merge into one line. Losing the overlap costs a cell per boundary
    /// and puts two parallel lines between every pair of panes.
    #[test]
    fn neighbouring_panes_share_one_dividing_line() {
        let rects = grid_rects(
            Rect::new(0, 0, 100, 40),
            GridSize {
                rows: 2,
                columns: 3,
            },
            6,
        );

        assert_eq!(rects[1].x, rects[0].right() - 1, "columns must overlap by 1");
        assert_eq!(rects[2].x, rects[1].right() - 1, "columns must overlap by 1");
        assert_eq!(rects[3].y, rects[0].bottom() - 1, "rows must overlap by 1");
    }

    /// The overlap only exists to be shared. A single pane has no neighbour to
    /// share with, so it must not reach outside the area it was given.
    #[test]
    fn a_lone_pane_stays_inside_its_area() {
        let area = Rect::new(2, 3, 40, 12);
        let rects = grid_rects(
            area,
            GridSize {
                rows: 1,
                columns: 1,
            },
            1,
        );

        assert_eq!(rects, vec![area]);
    }

    #[test]
    fn weighted_layout_allows_custom_columns() {
        let mut layout = GridLayout::new(GridSize {
            rows: 1,
            columns: 2,
        });
        layout.drag_divider(Divider::Column(0), Rect::new(0, 0, 100, 10), 70, 0);
        let rects = layout.rects(Rect::new(0, 0, 100, 10), 2);
        assert!(rects[0].width > rects[1].width);
    }

    #[test]
    fn resizing_layout_preserves_existing_weights() {
        let mut layout = GridLayout::new(GridSize {
            rows: 1,
            columns: 2,
        });
        layout.drag_divider(Divider::Column(0), Rect::new(0, 0, 100, 10), 70, 0);
        let original_columns = layout.column_weights.clone();

        layout.set_size(GridSize {
            rows: 2,
            columns: 3,
        });
        assert_eq!(layout.size().rows, 2);
        assert_eq!(layout.size().columns, 3);
        assert_eq!(&layout.column_weights[..2], &original_columns[..]);
        assert_eq!(layout.column_weights[2], 1000);

        layout.set_size(GridSize {
            rows: 1,
            columns: 1,
        });
        assert_eq!(layout.row_weights.len(), 1);
        assert_eq!(layout.column_weights.len(), 1);
    }
}
