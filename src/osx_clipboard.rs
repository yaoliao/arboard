use cocoa::appkit::NSPasteboardTypeString;
use cocoa::base::{id, nil};
use cocoa::foundation::{NSInteger, NSString};
#[cfg(feature = "image-data")]
use core_graphics::{
	base::{kCGBitmapByteOrderDefault, kCGImageAlphaLast, kCGRenderingIntentDefault, CGFloat},
	color_space::CGColorSpace,
	data_provider::{CGDataProvider, CustomData},
	image::CGImage,
};
use image::DynamicImage;
use log::{error, info};
use objc::runtime::{BOOL, YES};
use objc::{msg_send, sel, sel_impl};

use super::common::Error;
#[cfg(feature = "image-data")]
use super::common::ImageData;

pub const TIFF: &str = "public.tiff";
pub const FILE_URL: &str = "public.file-url";

pub struct OSXClipboardContext {
	pasteboard: cocoa::base::id,
}

impl OSXClipboardContext {
	pub fn new() -> Result<Self, Error> {
		let pasteboard = unsafe { cocoa::appkit::NSPasteboard::generalPasteboard(nil) };
		Ok(OSXClipboardContext { pasteboard })
	}

	pub(crate) fn get_text(&mut self) -> Result<String, Error> {
		unsafe {
			let pasteboard: id = self.pasteboard;
			let contents: id = msg_send![pasteboard, stringForType: NSPasteboardTypeString];
			if contents.is_null() {
				Err(Error::Unknown { description: "can not get string from clipboard".to_string() })
			} else {
				Ok(from_nsstring(contents))
			}
		}
	}

	pub(crate) fn set_text(&mut self, data: String) -> Result<(), Error> {
		unsafe {
			let nsstring = make_nsstring(data.as_str());
			let pasteboard: id = self.pasteboard;
			let _: NSInteger = msg_send![pasteboard, clearContents];
			let result: BOOL =
				msg_send![pasteboard, setString: nsstring forType: NSPasteboardTypeString];
			if result != YES {
				Err(Error::Unknown { description: "failed to set clipboard".to_string() })
			} else {
				Ok(())
			}
		}
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
		let available_type = available_type_names();
		info!("available_type : {:?}", available_type);

		if !available_type.contains(&String::from(TIFF)) {
			return Err(Error::Unknown { description: "probably not a picture".to_string() });
		}

		if available_type.contains(&String::from(FILE_URL)) {
			let pb_type = make_nsstring(FILE_URL);
			let data: id = unsafe { msg_send![self.pasteboard, dataForType: pb_type] };
			if data.is_null() {
				return Err(Error::Unknown { description: "can not get data".to_string() });
			}
			let data = from_nsdata(data);
			let file_url = String::from_utf8_lossy(&data);
			info!("img file url : {:?}", file_url);

			let file_url = file_url.strip_prefix("file://");
			if file_url.is_none() {
				return Err(Error::Unknown { description: "file url illegal".to_string() });
			}
			let decode_url = urlencoding::decode(file_url.unwrap());
			if decode_url.is_err() {
				return Err(Error::Unknown { description: "decode url error".to_string() });
			}

			// TODO deal unwrap
			let dyna_img = match image::io::Reader::open(decode_url.unwrap().into_owned())
				.unwrap()
				.with_guessed_format()
				.unwrap()
				.decode()
			{
				Ok(img) => img,
				Err(e) => {
					error!("open img error: {:?}", e);
					return Err(Error::Unknown { description: "open img error".to_string() });
				}
			};
			return deal_dynamic_image(dyna_img);
		}

		let pb_type = make_nsstring(TIFF);
		let data: id = unsafe { msg_send![self.pasteboard, dataForType: pb_type] };
		if data.is_null() {
			return Err(Error::Unknown { description: "can not get data".to_string() });
		}
		let data = from_nsdata(data);
		let reader =
			image::io::Reader::with_format(std::io::Cursor::new(data), image::ImageFormat::Tiff);
		return match reader.decode() {
			Ok(img) => deal_dynamic_image(img),
			Err(_) => Err(Error::ConversionFailure),
		};
	}

	#[cfg(feature = "image-data")]
	pub(crate) fn set_image(&mut self, data: ImageData) -> Result<(), Error> {
		use objc_foundation::INSArray;
		let pixels = data.bytes.into();
		let image = image_from_pixels(pixels, data.width, data.height)
			.map_err(|_| Error::ConversionFailure)?;
		let objects: objc_id::Id<
			objc_foundation::NSArray<objc_foundation::NSObject, objc_id::Owned>,
		> = objc_foundation::NSArray::from_vec(vec![image]);
		let _: usize = unsafe { msg_send![self.pasteboard, clearContents] };
		let success: BOOL = unsafe { msg_send![self.pasteboard, writeObjects: objects] };
		if success == objc::runtime::NO {
			return Err(Error::Unknown {
				description:
					"Failed to write the image to the pasteboard (`writeObjects` returned NO)."
						.into(),
			});
		}
		Ok(())
	}
}

fn deal_dynamic_image(dyna_img: DynamicImage) -> Result<ImageData<'static>, Error> {
	let rgba = dyna_img.into_rgba8();
	let (w, h) = rgba.dimensions();
	let img = ImageData { width: w as usize, height: h as usize, bytes: rgba.into_raw().into() };
	info!(
		"img: width: {:?}; height: {:?};  bytes len: {:?}",
		img.width,
		img.height,
		img.bytes.len()
	);
	Ok(img)
}

/// Returns an NSImage object on success.
#[cfg(feature = "image-data")]
fn image_from_pixels(
	pixels: Vec<u8>,
	width: usize,
	height: usize,
) -> Result<objc_id::Id<objc_foundation::NSObject>, Box<dyn std::error::Error>> {
	#[repr(C)]
	#[derive(Copy, Clone)]
	pub struct NSSize {
		pub width: CGFloat,
		pub height: CGFloat,
	}

	#[derive(Debug, Clone)]
	struct PixelArray {
		data: Vec<u8>,
	}

	impl CustomData for PixelArray {
		unsafe fn ptr(&self) -> *const u8 {
			self.data.as_ptr()
		}
		unsafe fn len(&self) -> usize {
			self.data.len()
		}
	}

	let colorspace = CGColorSpace::create_device_rgb();
	let bitmap_info: u32 = kCGBitmapByteOrderDefault | kCGImageAlphaLast;
	let pixel_data: Box<Box<dyn CustomData>> = Box::new(Box::new(PixelArray { data: pixels }));
	let provider = unsafe { CGDataProvider::from_custom_data(pixel_data) };
	let rendering_intent = kCGRenderingIntentDefault;
	let cg_image = CGImage::new(
		width,
		height,
		8,
		32,
		4 * width,
		&colorspace,
		bitmap_info,
		&provider,
		false,
		rendering_intent,
	);
	let size = NSSize { width: width as CGFloat, height: height as CGFloat };
	let nsimage_class = objc::runtime::Class::get("NSImage").ok_or("Class::get(\"NSImage\")")?;
	let image: objc_id::Id<objc_foundation::NSObject> =
		unsafe { objc_id::Id::from_ptr(msg_send![nsimage_class, alloc]) };
	let () = unsafe { msg_send![image, initWithCGImage:cg_image size:size] };
	Ok(image)
}

fn make_nsstring(s: &str) -> id {
	use cocoa::foundation::NSAutoreleasePool;
	unsafe { NSString::alloc(nil).init_str(s).autorelease() }
}

fn from_nsdata(data: id) -> Vec<u8> {
	unsafe {
		let len: cocoa::foundation::NSUInteger = msg_send![data, length];
		let bytes: *const std::ffi::c_void = msg_send![data, bytes];
		let mut out: Vec<u8> = Vec::with_capacity(len as usize);
		std::ptr::copy_nonoverlapping(bytes as *const u8, out.as_mut_ptr(), len as usize);
		out.set_len(len as usize);
		out
	}
}

fn from_nsstring(s: id) -> String {
	unsafe {
		let slice = std::slice::from_raw_parts(s.UTF8String() as *const _, s.len());
		let result = std::str::from_utf8_unchecked(slice);
		result.into()
	}
}

fn available_type_names() -> Vec<String> {
	use cocoa::foundation::NSArray;
	let res = unsafe {
		let pasteboard = cocoa::appkit::NSPasteboard::generalPasteboard(nil);
		let types: id = msg_send![pasteboard, types];
		let types_len = types.count() as usize;
		(0..types_len)
			.map(|i| from_nsstring(types.objectAtIndex(i as cocoa::foundation::NSUInteger)))
			.collect()
	};
	res
}
