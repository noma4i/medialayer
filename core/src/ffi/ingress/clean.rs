use super::command::{Declared, Query, Subscribing};
use crate::api::{clean_item, clean_item_known, clean_url, Known, MediaItem};
use std::sync::Arc;

/// The intake of every declared source: the one cleaning rule runs here, on the executor, never in a caller's constructor.
pub(crate) fn declared(command: Declared, known: &mut Known) -> (Declared, Vec<MediaItem>) {
    let mut wholes = Vec::new();
    let command = match command {
        Declared::PlaceOpen { owner, item } => Declared::PlaceOpen { owner, item: clean_item(item) },
        Declared::KeySeek { key, fraction } => Declared::KeySeek { key: clean_url(&key), fraction },
        Declared::VideoWindow { owner, active, sources } => Declared::VideoWindow { owner, active, sources: sources.map(|sources| video_sources(sources, known, &mut wholes)) },
        Declared::ImageWindow { owner, active, pages } => Declared::ImageWindow { owner, active, pages: pages.map(|pages| image_pages(pages, known, &mut wholes)) },
        Declared::WholeAdd { owner, items } => Declared::WholeAdd { owner, items: items.into_iter().map(clean_item).collect() },
        Declared::WholeRemove { owner, items } => Declared::WholeRemove { owner, items: items.into_iter().map(clean_item).collect() },
        Declared::Subscribe { owner, subscribing } => Declared::Subscribe { owner, subscribing: subscribing_to(subscribing) },
        Declared::Query { query, reply } => Declared::Query { query: query_of(query), reply },
        other => other,
    };
    let scale = wholes_scale(&command);
    if scale > 0 && known.len() > 4 * scale + 4096 {
        known.clear();
    }
    (command, wholes)
}

fn wholes_scale(command: &Declared) -> usize {
    match command {
        Declared::VideoWindow { sources: Some(sources), .. } => sources.len(),
        Declared::ImageWindow { pages: Some(pages), .. } => pages.iter().map(Vec::len).sum(),
        _ => 0,
    }
}

fn subscribing_to(subscribing: Subscribing) -> Subscribing {
    match subscribing {
        Subscribing::State { key, listener } => Subscribing::State { key: clean_url(&key), listener },
        Subscribing::Tone { item, band, listener } => Subscribing::Tone { item: clean_item(item), band, listener },
        other @ (Subscribing::Stats { .. } | Subscribing::Playback { .. }) => other,
    }
}

fn query_of(query: Query) -> Query {
    match query {
        Query::Retry(item) => Query::Retry(clean_item(item)),
        Query::ReadRoute(item) => Query::ReadRoute(clean_item(item)),
        Query::Refresh(item, route) => Query::Refresh(clean_item(item), route),
        Query::Cached(item) => Query::Cached(clean_item(item)),
        Query::Tone(item, band) => Query::Tone(clean_item(item), band),
        Query::Pixels(item) => Query::Pixels(clean_item(item)),
        Query::Placeholder(item, ready) => Query::Placeholder(clean_item(item), ready),
        Query::Image { url, headers } => Query::Image { url: clean_url(&url), headers },
        Query::ImageSize { url, headers } => Query::ImageSize { url: clean_url(&url), headers },
        other @ (Query::Pool | Query::Decode(_) | Query::Capture(_)) => other,
    }
}

fn video_sources(mut sources: Arc<Vec<Option<MediaItem>>>, known: &mut Known, wholes: &mut Vec<MediaItem>) -> Arc<Vec<Option<MediaItem>>> {
    for item in Arc::make_mut(&mut sources).iter_mut().flatten() {
        clean_item_known(item, known, wholes);
    }
    sources
}

fn image_pages(mut pages: Arc<Vec<Vec<MediaItem>>>, known: &mut Known, wholes: &mut Vec<MediaItem>) -> Arc<Vec<Vec<MediaItem>>> {
    for item in Arc::make_mut(&mut pages).iter_mut().flatten() {
        clean_item_known(item, known, wholes);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Kind;
    use crate::prefetch::CacheStrategy;

    fn list(count: usize) -> Arc<Vec<Option<MediaItem>>> {
        Arc::new((0..count).map(|index| Some(MediaItem { strategy: if index % 2 == 0 { CacheStrategy::Whole } else { CacheStrategy::Window }, ..MediaItem::bare(&format!("https://o.test/{index}.mp4"), Kind::Video) })).collect())
    }

    fn replace_cost(count: usize, known: &mut Known) -> (usize, usize) {
        let first = Declared::VideoWindow { owner: 1, active: 0, sources: Some(list(count)) };
        declared(first, known);
        let second = Declared::VideoWindow { owner: 1, active: 0, sources: Some(list(count)) };
        let mut fresh = 0;
        let allocations = crate::alloc_probe::allocations_of(|| fresh = declared(second, known).1.len());
        (allocations, fresh)
    }

    #[test]
    fn inv_api_69_replacing_a_list_of_unchanged_items_allocates_nothing_that_grows_with_its_length() {
        let (short, short_fresh) = replace_cost(200, &mut Known::new());
        let (long, long_fresh) = replace_cost(8000, &mut Known::new());
        assert_eq!((short_fresh, long_fresh), (0, 0), "an item handed over before is not handed over again");
        assert!(long <= short + 8, "unchanged items allocated {long} times for 8000 items and {short} for 200");
    }

    #[test]
    fn inv_api_69_a_new_whole_item_is_handed_over_once_and_cleaned_like_before() {
        let mut known = Known::new();
        let first = Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(vec![Some(MediaItem { strategy: CacheStrategy::Whole, ..MediaItem::bare(" https://o.test/a.mp4 ", Kind::Video) }), Some(MediaItem::bare("ftp://bad", Kind::Video))])) };
        let (Declared::VideoWindow { sources: Some(sources), .. }, wholes) = declared(first, &mut known) else { unreachable!() };
        assert_eq!(wholes.len(), 1);
        assert_eq!(sources[0].as_ref().unwrap().url, "https://o.test/a.mp4");
        assert_eq!(sources[1].as_ref().unwrap().url, "");
    }
}
