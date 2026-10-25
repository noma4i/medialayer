use crate::api::MediaItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RowLayout {
    rows_per_page: u32,
    reversed: bool,
    rows: u32,
    active: u32,
}

impl RowLayout {
    pub fn new(rows_per_page: u32, reversed: bool) -> RowLayout {
        RowLayout { rows_per_page, reversed, rows: 0, active: 0 }
    }

    pub fn active(&self) -> u32 {
        self.active
    }

    pub fn move_to(&mut self, page: u32) {
        self.active = page;
    }

    pub fn paged(&mut self, mut rows: Vec<Vec<MediaItem>>) -> Vec<Vec<MediaItem>> {
        self.rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        if self.reversed {
            rows.reverse();
        }
        let mut pages = Vec::with_capacity(rows.len().div_ceil(self.rows_per_page as usize));
        let mut rows = rows.into_iter();
        while let Some(mut page) = rows.next() {
            for row in rows.by_ref().take(self.rows_per_page as usize - 1) {
                page.extend(row);
            }
            pages.push(page);
        }
        pages
    }

    pub fn show(&mut self, first: u32, last: u32) {
        self.active = self.page_of(first).min(self.page_of(last));
    }

    fn page_of(&self, row: u32) -> u32 {
        let row = row.min(self.rows.saturating_sub(1));
        if self.reversed { (self.rows.saturating_sub(1) - row) / self.rows_per_page } else { row / self.rows_per_page }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Kind;

    fn rows(counts: &[usize]) -> Vec<Vec<MediaItem>> {
        let mut next = 0;
        counts.iter().map(|count| (0..*count).map(|_| { next += 1; MediaItem::bare(&format!("https://o.test/{next}.jpg"), Kind::Image) }).collect()).collect()
    }

    fn urls(pages: &[Vec<MediaItem>]) -> Vec<Vec<String>> {
        pages.iter().map(|page| page.iter().map(|item| item.url.rsplit('/').next().unwrap().to_string()).collect()).collect()
    }

    #[test]
    fn inv_window_03_the_first_visible_row_picks_the_page_and_empty_rows_keep_their_page() {
        let mut layout = RowLayout::new(2, false);
        let pages = layout.paged(rows(&[1, 0, 2, 1, 0]));
        assert_eq!(urls(&pages), vec![vec!["1.jpg"], vec!["2.jpg", "3.jpg", "4.jpg"], vec![]]);

        layout.show(3, 4);

        assert_eq!(layout.active(), 1);
    }

    #[test]
    fn inv_window_04_a_reversed_list_counts_pages_from_its_last_row_and_picks_by_the_last_visible_row() {
        let mut layout = RowLayout::new(2, true);
        let pages = layout.paged(rows(&[1, 1, 1, 1, 1]));
        assert_eq!(urls(&pages), vec![vec!["5.jpg", "4.jpg"], vec!["3.jpg", "2.jpg"], vec!["1.jpg"]]);

        layout.show(1, 3);

        assert_eq!(layout.active(), 0);
        layout.show(0, 1);
        assert_eq!(layout.active(), 1);
    }
}
