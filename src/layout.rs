use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub rows: usize,
    pub columns: usize,
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
        let count = count.clamp(1, 100);
        let columns = (count as f64).sqrt().ceil() as usize;
        let rows = count.div_ceil(columns);
        Self { rows, columns }
    }

    pub fn new(rows: usize, columns: usize) -> Option<Self> {
        if rows == 0 || columns == 0 || rows * columns > 100 {
            return None;
        }
        Some(Self { rows, columns })
    }

    pub fn count(self) -> usize {
        self.rows * self.columns
    }
}

pub fn grid_rects(area: Rect, grid: GridSize, count: usize) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(count);
    if grid.rows == 0 || grid.columns == 0 {
        return rects;
    }

    let width_base = area.width / grid.columns as u16;
    let width_extra = area.width % grid.columns as u16;
    let height_base = area.height / grid.rows as u16;
    let height_extra = area.height % grid.rows as u16;

    let mut y = area.y;
    for row in 0..grid.rows {
        let row_height = height_base + u16::from((row as u16) < height_extra);
        let mut x = area.x;
        for column in 0..grid.columns {
            if rects.len() >= count {
                break;
            }

            let column_width = width_base + u16::from((column as u16) < width_extra);
            rects.push(Rect {
                x,
                y,
                width: column_width,
                height: row_height,
            });
            x = x.saturating_add(column_width);
        }
        y = y.saturating_add(row_height);
    }

    rects
}

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

    #[test]
    fn grid_rects_cover_area() {
        let rects = grid_rects(
            Rect::new(0, 0, 100, 40),
            GridSize {
                rows: 2,
                columns: 3,
            },
            6,
        );
        assert_eq!(rects.len(), 6);
        assert_eq!(rects[0].height + rects[3].height, 40);
        assert_eq!(rects[0].width + rects[1].width + rects[2].width, 100);
    }
}
