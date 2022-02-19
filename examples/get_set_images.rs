use arboard::{Clipboard, ImageData};

pub fn main() {
	let mut clip_board = Clipboard::new().unwrap();
	let img = clip_board.get_image().unwrap();
	println!(
		"img: width: {:?}; height: {:?};  bytes len: {:?}",
		img.width,
		img.height,
		img.bytes.len()
	);

	let mut clip_board = Clipboard::new().unwrap();
	clip_board.set_image(img).unwrap();
}
