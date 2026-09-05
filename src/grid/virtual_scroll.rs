use crate::db::types::SqlValue;

pub struct VirtualWindow {
    pub offset: i64,
    pub rows: Vec<Vec<SqlValue>>,
    pub rowids: Vec<Option<i64>>,
    pub total_rows: i64,
    pub viewport_rows: usize,
    pub fetch_in_flight: bool,
    pub tick_count: u64,
}

impl VirtualWindow {
    pub fn new(offset: i64, rows: Vec<Vec<SqlValue>>, total_rows: i64) -> Self {
        Self {
            offset,
            rows,
            rowids: Vec::new(),
            total_rows,
            viewport_rows: 20,
            fetch_in_flight: false,
            tick_count: 0,
        }
    }

    pub fn get_rowid(&self, abs_row: i64) -> Option<i64> {
        if abs_row < self.offset || self.get_row(abs_row).is_none() {
            return None;
        }
        self.rowids
            .get((abs_row - self.offset) as usize)
            .copied()
            .flatten()
    }

    pub fn get_row(&self, abs_row: i64) -> Option<&Vec<SqlValue>> {
        if abs_row < self.offset {
            return None;
        }
        let idx = (abs_row - self.offset) as usize;
        self.rows.get(idx)
    }

    pub fn needs_prefetch(&self, focused_row: i64) -> bool {
        let window_end = self.offset + self.rows.len() as i64;
        let vp = self.viewport_rows as i64;
        focused_row < self.offset + 10 || focused_row + vp + 20 > window_end
    }

    pub fn fetch_params(&self, focused_row: i64) -> (i64, i64) {
        let start = (focused_row - 50).max(0);
        let end = focused_row + self.viewport_rows as i64 + 50;
        let limit = (end - start).max(100);
        (start, limit)
    }
}
