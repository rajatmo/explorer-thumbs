//! Draws a PDF's first-page thumbnail to a .bmp file, without File Explorer:
//!
//!     cargo run --release -p pdfcraft-explorer-thumbs --example thumb -- in.pdf out.bmp 256
//!
//! The picture is exactly what the handler gives Explorer for that size.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        eprintln!("usage: thumb <in.pdf> <out.bmp> [side, default 256]");
        return ExitCode::FAILURE;
    };
    let side = rest.first().and_then(|s| s.parse().ok()).unwrap_or(256);
    let bytes = match std::fs::read(input) {
        Ok(bytes) => bytes,
        Err(why) => {
            eprintln!("{input}: {why}");
            return ExitCode::FAILURE;
        }
    };
    let thumb = match pdfcraft_explorer_thumbs::first_page(bytes, side) {
        Ok(thumb) => thumb,
        Err(why) => {
            eprintln!("{input}: {why}");
            return ExitCode::FAILURE;
        }
    };
    // A 32-bit top-down BMP: a 14-byte file header, a 40-byte info header, then the pixels.
    let mut bmp = Vec::with_capacity(54 + thumb.bgra.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + thumb.bgra.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(thumb.width as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(thumb.height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    bmp.extend_from_slice(&thumb.bgra);
    if let Err(why) = std::fs::write(output, bmp) {
        eprintln!("{output}: {why}");
        return ExitCode::FAILURE;
    }
    println!("{output}: {} × {} pixels", thumb.width, thumb.height);
    ExitCode::SUCCESS
}
