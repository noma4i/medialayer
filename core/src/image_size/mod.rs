mod exif;
#[cfg(test)]
pub(crate) mod fixtures;
mod parse;

pub use parse::parse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl ImageSize {
    pub fn encode(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    pub fn decode(text: &str) -> Option<ImageSize> {
        let (width, height) = text.split_once('x')?;
        let size = ImageSize { width: width.parse().ok()?, height: height.parse().ok()? };
        (size.width > 0 && size.height > 0).then_some(size)
    }

    fn swapped(self) -> ImageSize {
        ImageSize { width: self.height, height: self.width }
    }
}

#[cfg(test)]
mod size_tests;
