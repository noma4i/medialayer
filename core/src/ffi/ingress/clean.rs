use super::command::{Declared, Query, Subscribing};
use crate::api::{clean_item, clean_url, MediaItem};
use std::sync::Arc;

/// The intake of every declared source: the one cleaning rule runs here, on the executor, never in a caller's constructor.
pub(crate) fn declared(command: Declared) -> Declared {
    match command {
        Declared::PlaceOpen { owner, item } => Declared::PlaceOpen { owner, item: clean_item(item) },
        Declared::KeySeek { key, fraction } => Declared::KeySeek { key: clean_url(&key), fraction },
        Declared::VideoWindow { owner, active, sources } => Declared::VideoWindow { owner, active, sources: sources.map(video_sources) },
        Declared::ImageWindow { owner, active, pages } => Declared::ImageWindow { owner, active, pages: pages.map(image_pages) },
        Declared::WholeAdd { owner, items } => Declared::WholeAdd { owner, items: items.into_iter().map(clean_item).collect() },
        Declared::WholeRemove { owner, items } => Declared::WholeRemove { owner, items: items.into_iter().map(clean_item).collect() },
        Declared::Subscribe { owner, subscribing } => Declared::Subscribe { owner, subscribing: subscribing_to(subscribing) },
        Declared::Query { query, reply } => Declared::Query { query: query_of(query), reply },
        other => other,
    }
}

fn subscribing_to(subscribing: Subscribing) -> Subscribing {
    match subscribing {
        Subscribing::State { key, listener } => Subscribing::State { key: clean_url(&key), listener },
        Subscribing::Tone { item, band, listener } => Subscribing::Tone { item: clean_item(item), band, listener },
        stats @ Subscribing::Stats { .. } => stats,
    }
}

fn query_of(query: Query) -> Query {
    match query {
        Query::PlayUrl(item) => Query::PlayUrl(clean_item(item)),
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

fn video_sources(sources: Arc<Vec<Option<MediaItem>>>) -> Arc<Vec<Option<MediaItem>>> {
    Arc::new(sources.iter().cloned().map(|source| source.map(clean_item)).collect())
}

fn image_pages(pages: Arc<Vec<Vec<MediaItem>>>) -> Arc<Vec<Vec<MediaItem>>> {
    Arc::new(pages.iter().map(|page| page.iter().cloned().map(clean_item).collect()).collect())
}
