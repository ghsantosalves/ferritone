use std::path::Path;

use lofty::{
    config::ParseOptions,
    file::{AudioFile, TaggedFileExt},
    picture::PictureType,
    probe::Probe,
    tag::Accessor,
};

#[derive(Default)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

pub fn read_track(path: &Path) -> TrackInfo {
    let Ok(file) = Probe::open(path).and_then(|probe| {
        probe
            .options(ParseOptions::new().read_cover_art(false))
            .read()
    }) else {
        return TrackInfo::default();
    };
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let text = |value: Option<std::borrow::Cow<'_, str>>| {
        value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let duration = file.properties().duration().as_millis() as u64;
    TrackInfo {
        title: text(tag.and_then(Accessor::title)),
        artist: text(tag.and_then(Accessor::artist)),
        album: text(tag.and_then(Accessor::album)),
        duration_ms: (duration > 0).then_some(duration),
    }
}

pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    let file = Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .ok()?;
    let pictures = file.tags().iter().flat_map(|tag| tag.pictures());
    pictures
        .clone()
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.into_iter().next())
        .map(|picture| picture.data().to_vec())
}

pub struct DecodedImage {
    pixels: gtk::glib::Bytes,
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
}

impl DecodedImage {
    fn from_pixbuf(image: &gtk::gdk_pixbuf::Pixbuf) -> Self {
        Self {
            pixels: image.read_pixel_bytes(),
            width: image.width(),
            height: image.height(),
            rowstride: image.rowstride(),
            alpha: image.has_alpha(),
        }
    }

    pub fn pixbuf(self) -> gtk::gdk_pixbuf::Pixbuf {
        gtk::gdk_pixbuf::Pixbuf::from_bytes(
            &self.pixels,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            self.alpha,
            8,
            self.width,
            self.height,
            self.rowstride,
        )
    }
}

pub fn read_cover_image(path: &Path) -> Option<(DecodedImage, Option<DecodedImage>)> {
    use gtk::{gdk_pixbuf::PixbufLoader, prelude::*};
    let encoded = read_cover(path)?;
    let loader = PixbufLoader::new();
    loader.connect_size_prepared(|loader, width, height| {
        let side = width.max(height);
        if side > 1440 {
            let scale = 1440.0 / f64::from(side);
            loader.set_size(
                (f64::from(width) * scale).round().max(1.0) as i32,
                (f64::from(height) * scale).round().max(1.0) as i32,
            );
        }
    });
    let written = loader.write(&encoded);
    let closed = loader.close();
    written.ok()?;
    closed.ok()?;
    let image = loader.pixbuf()?;
    let thumbnail = crate::scale_image(&image, 96)
        .as_ref()
        .map(DecodedImage::from_pixbuf);
    Some((DecodedImage::from_pixbuf(&image), thumbnail))
}
