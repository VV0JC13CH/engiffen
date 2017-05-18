//! Engiffen is a library to convert sequences of images into animated Gifs.
//!
//! This library is a wrapper around the image and gif crates to convert
//! a sequence of images into an animated Gif.

#![doc(html_root_url = "https://docs.rs/engiffen/0.3.0")]

extern crate image;
extern crate gif;
extern crate color_quant;

use std::io::{self, Write};
use std::{error, fmt};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use image::{GenericImage, DynamicImage};
use gif::{Frame, Encoder, Repeat, SetParameter};
use color_quant::NeuQuant;

#[cfg(feature = "debug-stderr")] use std::time::{Instant};

#[cfg(feature = "debug-stderr")]
fn ms(duration: Instant) -> u64 {
    let duration = duration.elapsed();
    duration.as_secs() * 1000 + duration.subsec_nanos() as u64 / 1000000
}

/// An image, currently a wrapper around `image::DynamicImage`. If loaded from
/// disk through the `load_image` or `load_images` functions, its path property
/// contains the path used to read it from disk.
pub struct Image {
    inner: DynamicImage,
    pub path: Option<PathBuf>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Image {{ path: {:?}, dimensions: {} x {} }}", self.path, self.inner.width(), self.inner.height())
    }
}

#[derive(Debug)]
pub enum Error {
    NoImages,
    Mismatch((u32, u32), (u32, u32)),
    ImageLoad(image::ImageError),
    ImageWrite(io::Error),
}

impl From<image::ImageError> for Error {
    fn from(err: image::ImageError) -> Error {
        Error::ImageLoad(err)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::ImageWrite(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::NoImages => write!(f, "No frames sent for engiffening"),
            Error::Mismatch(_, _) => write!(f, "Frames don't have the same dimensions"),
            Error::ImageLoad(ref e) => write!(f, "Image load error: {}", e),
            Error::ImageWrite(ref e) => write!(f, "Image write error: {}", e),
        }
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::NoImages => "No frames sent for engiffening",
            Error::Mismatch(_, _) => "Frames don't have the same dimensions",
            Error::ImageLoad(_) => "Unable to load image",
            Error::ImageWrite(_) => "Unable to write image",
        }
    }
}

/// Struct representing an animated Gif
#[derive(Eq, PartialEq, Clone, Hash)]
pub struct Gif {
    pub palette: Vec<u8>,
    pub transparency: Option<u8>,
    pub width: u16,
    pub height: u16,
    pub images: Vec<Vec<u8>>,
    pub delay: u16,
}

impl fmt::Debug for Gif {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Gif {{ palette: Vec<u8 x {:?}>, transparency: {:?}, width: {:?}, height: {:?}, images: Vec<Vec<u8> x {:?}>, delay: {:?} }}",
            self.palette.len(),
            self.transparency,
            self.width,
            self.height,
            self.images.len(),
            self.delay
        )
    }
}

impl Gif {
    /// Writes the animated Gif to any output that implements Write.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::fs::File;
    /// # use engiffen::{Image, engiffen};
    /// # fn foo() -> Result<(), engiffen::Error> {
    /// # let images: Vec<Image> = vec![];
    /// let mut output = File::create("output.gif")?;
    /// let gif = engiffen(&images, 10, None)?;
    /// gif.write(&mut output)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the `std::io::Result` of the underlying `write` function calls.
    pub fn write<W: io::Write>(&self, mut out: &mut W) -> Result<(), Error> {
        let mut encoder = Encoder::new(&mut out, self.width, self.height, &self.palette)?;
        encoder.set(Repeat::Infinite)?;
        for img in &self.images {
            let mut frame = Frame::default();
            frame.delay = self.delay / 10;
            frame.width = self.width;
            frame.height = self.height;
            frame.buffer = Cow::Borrowed(&*img);
            frame.transparent = self.transparency;
            encoder.write_frame(&frame)?;
        }
        Ok(())
    }
}

/// Loads an image from the given file path.
///
/// # Examples
///
/// ```rust,no_run
/// # use engiffen::{load_image, Image, Error};
/// # use std::path::PathBuf;
/// # fn foo() -> Result<Image, Error> {
/// let image = load_image("test/ball/ball01.bmp")?;
/// assert_eq!(image.path, Some(PathBuf::from("test/ball/ball01.bmp")));
/// # Ok(image)
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the path can't be read or if the image can't be decoded
pub fn load_image<P>(path: P) -> Result<Image, Error>
    where P: AsRef<Path> {
    let img = image::open(&path)?;
    Ok(Image {
        inner: img,
        path: Some(path.as_ref().to_path_buf()),
    })
}

/// Loads images from a list of given paths. Errors encountered while loading files
/// are skipped.
///
/// # Examples
///
/// ```rust,no_run
/// # use engiffen::load_images;
/// let paths = vec!["tests/ball/ball06.bmp", "tests/ball/ball07.bmp", "tests/ball/ball08.bmp"];
/// let images = load_images(&paths);
/// assert_eq!(images.len(), 2); // The last path doesn't exist. It was silently skipped.
/// ```
///
/// Skips images that fail to load. If all images fail, returns an empty vector.
pub fn load_images<P>(paths: &[P]) -> Vec<Image>
    where P: AsRef<Path> {
    paths.iter()
        .map(|path| load_image(path))
        .filter_map(|img| img.ok())
        .collect()
}

/// Converts a sequence of images into a `Gif` at a given frame rate. The `sample_rate`
/// parameter, if passed, specifies the fraction of pixels that will be sampled
/// when the color palette is computed. Higher values means fewer pixels sampled, and
/// it scales quadratically. In practice, if the value is `None` or `Some(1)`, then all
/// pixels will be sampled. Otherwise, for values of `Some(N)`, every Nth pixel on every
/// Nth row will be sampled, for a total of 1/(N*N) of the pixels sampled.
///
/// # Examples
///
/// ```rust,no_run
/// # use engiffen::{load_images, engiffen, Gif, Error};
/// # fn foo() -> Result<Gif, Error> {
/// let paths = vec!["tests/ball/ball01.bmp", "tests/ball/ball02.bmp", "tests/ball/ball03.bmp"];
/// let images = load_images(&paths);
/// let gif = engiffen(&images, 10, None)?;
/// assert_eq!(gif.images.len(), 3);
/// # Ok(gif)
/// # }
/// ```
///
/// # Errors
///
/// If any image dimensions differ, this function will return an Error::Mismatch
/// containing tuples of the conflicting image dimensions.
pub fn engiffen(imgs: &[Image], fps: usize, sample_rate: Option<u32>) -> Result<Gif, Error> {
    if imgs.is_empty() {
        return Err(Error::NoImages);
    }
    #[cfg(feature = "debug-stderr")] let time_check_dimensions = Instant::now();
    let (width, height) = {
        let ref first = imgs[0].inner;
        let first_dimensions = (first.width(), first.height());
        for img in imgs.iter() {
            let other_dimensions = (img.inner.width(), img.inner.height());
            if first_dimensions != other_dimensions {
                return Err(Error::Mismatch(first_dimensions, other_dimensions));
            }
        }
        first_dimensions
    };
    #[cfg(feature = "debug-stderr")]
    writeln!(&mut std::io::stderr(), "Checked image dimensions in {} ms.", ms(time_check_dimensions)).expect("failed to write to stderr");
    #[cfg(feature = "debug-stderr")] let time_push = Instant::now();
    let mut colors: Vec<u8> = Vec::with_capacity(width as usize * height as usize * imgs.len());
    let skip_pixels = sample_rate.unwrap_or(1);
    for img in imgs.iter() {
        for (x, y, px) in img.inner.pixels() {
            if skip_pixels > 1 {
                if x % skip_pixels != 0 || y % skip_pixels != 0 {
                    continue;
                }
            }
            if px.data[3] == 0 {
                colors.push(0);
                colors.push(0);
                colors.push(0);
                colors.push(0);
            } else {
                colors.push(px.data[0]);
                colors.push(px.data[1]);
                colors.push(px.data[2]);
                colors.push(255);
            }
        }
    }
    #[cfg(feature = "debug-stderr")]
    writeln!(&mut std::io::stderr(), "Pushed all frame pixels in {} ms.", ms(time_push)).expect("failed to write to stderr");

    #[cfg(feature = "debug-stderr")] let time_quant = Instant::now();
    let quant = NeuQuant::new(10, 256, &colors);
    #[cfg(feature = "debug-stderr")]
    writeln!(&mut std::io::stderr(), "Computed palette in {} ms.", ms(time_quant)).expect("failed to write to stderr");

    #[cfg(feature = "debug-stderr")] let time_map = Instant::now();
    let mut transparency = None;
    let mut cache: HashMap<[u8; 4], u8> = HashMap::new();
    let palettized_imgs: Vec<Vec<u8>> = imgs.iter().map(|img| {
        img.inner.pixels().map(|(_, _, px)| {
            *cache.entry(px.data).or_insert_with(|| {
                let idx = quant.index_of(&px.data) as u8;
                if px.data[3] == 0 { transparency = Some(idx); }
                idx
            })
        }).collect()
    }).collect();
    #[cfg(feature = "debug-stderr")]
    writeln!(&mut std::io::stderr(), "Mapped pixels to palette in {} ms.", ms(time_map)).expect("failed to write to stderr");

    let delay = (1000 / fps) as u16;

    Ok(Gif {
        palette: quant.color_map_rgb(),
        transparency: transparency,
        width: width as u16,
        height: height as u16,
        images: palettized_imgs,
        delay: delay,
    })
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::{load_image, engiffen, Error};
    use std::fs::{read_dir, File};

    #[test]
    fn test_error_on_size_mismatch() {
        let imgs: Vec<_> = read_dir("tests/mismatched_size").unwrap()
        .map(|e| e.unwrap().path())
        .map(|path| load_image(&path).unwrap())
        .collect();

        let res = engiffen(&imgs, 30, None);

        assert!(res.is_err());
        match res {
            Err(Error::Mismatch(one, another)) => {
                assert_eq!((one, another), ((100, 100), (50, 50)));
            },
            _ => unreachable!(),
        }
    }

    #[test] #[ignore]
    fn test_compress_palette() {
        // This takes a while to run when not in --release
        let imgs: Vec<_> = read_dir("tests/ball").unwrap()
            .map(|e| e.unwrap().path())
            .filter(|path| match path.extension() {
                Some(ext) if ext == "bmp" => true,
                _ => false,
            })
            .map(|path| load_image(&path).unwrap())
            .collect();

        let mut out = File::create("tests/ball.gif").unwrap();
        let gif = engiffen(&imgs, 10, Some(2));
        match gif {
            Ok(gif) => gif.write(&mut out),
            Err(_) => panic!("Test should have successfully made a gif."),
        };
    }

    #[test] #[ignore]
    fn test_simple_paletted_gif() {
        let imgs: Vec<_> = read_dir("tests/shrug").unwrap()
            .map(|e| e.unwrap().path())
            .filter(|path| match path.extension() {
                Some(ext) if ext == "tga" => true,
                _ => false,
            })
            .map(|path| load_image(&path).unwrap())
            .collect();

        let mut out = File::create("tests/shrug.gif").unwrap();
        let gif = engiffen(&imgs, 30, Some(2));
        match gif {
            Ok(gif) => gif.write(&mut out),
            Err(_) => panic!("Test should have successfully made a gif."),
        };
    }
}
