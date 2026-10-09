use std::ops::Range;

pub trait Keyed {
    fn key(&self) -> &str;
}

impl Keyed for String {
    fn key(&self) -> &str {
        self
    }
}

pub trait PageKeys {
    fn count(&self) -> usize;
    fn key(&self, page: usize) -> Option<&str>;
}

impl PageKeys for [Option<String>] {
    fn count(&self) -> usize {
        self.len()
    }

    fn key(&self, page: usize) -> Option<&str> {
        self.get(page)?.as_deref()
    }
}

impl<const N: usize> PageKeys for [Option<String>; N] {
    fn count(&self) -> usize {
        N
    }

    fn key(&self, page: usize) -> Option<&str> {
        self.get(page)?.as_deref()
    }
}

impl PageKeys for Vec<Option<String>> {
    fn count(&self) -> usize {
        self.len()
    }

    fn key(&self, page: usize) -> Option<&str> {
        self.get(page)?.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sliced<T> {
    first: usize,
    count: usize,
    items: Vec<Option<T>>,
}

impl<T> Sliced<T> {
    pub fn whole(items: Vec<Option<T>>) -> Sliced<T> {
        Sliced { first: 0, count: items.len(), items }
    }

    pub fn of(items: Vec<Option<T>>, range: Range<usize>) -> Sliced<T> {
        let count = items.len();
        let first = range.start.min(count);
        let end = range.end.min(count);
        Sliced { first, count, items: items.into_iter().skip(first).take(end - first).collect() }
    }

    pub fn map<U>(&self, convert: impl Fn(&T) -> Option<U>) -> Sliced<U> {
        Sliced { first: self.first, count: self.count, items: self.items.iter().map(|item| item.as_ref().and_then(&convert)).collect() }
    }

    pub fn present(&self) -> impl Iterator<Item = &T> {
        self.items.iter().flatten()
    }

    pub fn get(&self, page: usize) -> Option<&T> {
        self.items.get(page.checked_sub(self.first)?)?.as_ref()
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn into_present(self) -> impl Iterator<Item = T> {
        self.items.into_iter().flatten()
    }
}

impl<T> From<Vec<Option<T>>> for Sliced<T> {
    fn from(items: Vec<Option<T>>) -> Sliced<T> {
        Sliced::whole(items)
    }
}

impl<T> From<Vec<Vec<T>>> for Sliced<Vec<T>> {
    fn from(pages: Vec<Vec<T>>) -> Sliced<Vec<T>> {
        Sliced::whole(pages.into_iter().map(Some).collect())
    }
}

impl<T: Keyed> PageKeys for Sliced<T> {
    fn count(&self) -> usize {
        self.count
    }

    fn key(&self, page: usize) -> Option<&str> {
        self.get(page).map(Keyed::key)
    }
}
