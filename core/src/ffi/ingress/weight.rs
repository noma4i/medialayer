use crate::api::MediaItem;
use std::collections::HashMap;
use std::mem::size_of;

/// Retained heap of a value, counted by backing capacities. `None` means the arithmetic overflowed.
pub(crate) type Bytes = Option<u64>;

pub(crate) fn sum(parts: impl IntoIterator<Item = Bytes>) -> Bytes {
    parts.into_iter().try_fold(0u64, |total, part| total.checked_add(part?))
}

pub(crate) fn string(value: &String) -> Bytes {
    Some(value.capacity() as u64)
}

pub(crate) fn slots<T>(capacity: usize) -> Bytes {
    (capacity as u64).checked_mul(size_of::<T>() as u64)
}

pub(crate) fn headers(headers: &HashMap<String, String>) -> Bytes {
    let table = (headers.capacity() as u64).checked_mul((size_of::<(String, String)>() + 1) as u64)?;
    sum(std::iter::once(Some(table)).chain(headers.iter().flat_map(|(name, value)| [string(name), string(value)])))
}

#[cfg(test)]
thread_local! {
    pub(crate) static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn item_heap(item: &MediaItem) -> Bytes {
    #[cfg(test)]
    VISITS.with(|visits| visits.set(visits.get() + 1));
    sum([string(&item.url), headers(&item.headers), item.cover.as_ref().map_or(Some(0), string), item.blur_hash.as_ref().map_or(Some(0), string)])
}

pub(crate) fn item(item: &MediaItem) -> Bytes {
    sum([Some(size_of::<MediaItem>() as u64), item_heap(item)])
}

pub(crate) fn sources(sources: &Vec<Option<MediaItem>>) -> Bytes {
    sum(std::iter::once(slots::<Option<MediaItem>>(sources.capacity())).chain(sources.iter().flatten().map(item_heap)))
}

pub(crate) fn pages(pages: &Vec<Vec<MediaItem>>) -> Bytes {
    sum(std::iter::once(slots::<Vec<MediaItem>>(pages.capacity())).chain(pages.iter().map(|page| {
        sum(std::iter::once(slots::<MediaItem>(page.capacity())).chain(page.iter().map(item_heap)))
    })))
}
