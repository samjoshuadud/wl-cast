use std::{env, error::Error, process::Command, time::Instant};

use ffmpeg_next::{self as ffmpeg, Packet, codec, decoder, format::Pixel, frame};
use wl_cast::{Encoder, EncoderConfig, Frame};

const ENCODERS: [&str; 4] = ["openh264", "x264", "vaapi", "nvenc"];
const DEFAULT_SIZE: (usize, usize) = (1920, 1080);
const DEFAULT_FRAMES: usize = 120;
const FRAME_RATE: u32 = 60;
const MEMORY_BUDGET: usize = 400_000_000;
const MAX_DISTINCT_FRAMES: usize = 30;
const PSNR_EVERY: usize = 4;
const FLAT_COLORS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [128, 128, 128]];
const GLYPH_WIDTH: usize = 8;
const GLYPH_HEIGHT: usize = 14;
const SCROLL_PER_FRAME: usize = 3;
const BLOCK_SIZE: usize = 160;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        return child(&args[1..]);
    }

    let (width, height, frames) = parse(&args)?;
    println!("{width}x{height}, {frames} frames at {FRAME_RATE} fps, automatic bitrate");
    println!("encoder      fps  ms/frame     p95  cores  Mbit/s  PSNR dB  empty  err 709  err 601");
    for name in ENCODERS {
        let output = Command::new(env::current_exe()?)
            .env("WL_CAST_ENCODER", name)
            .args([
                "--child",
                &width.to_string(),
                &height.to_string(),
                &frames.to_string(),
            ])
            .output()?;
        if output.status.success() {
            print!("{}", String::from_utf8_lossy(&output.stdout));
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.lines().last().unwrap_or("failed");
            println!("{name:<10}   unavailable: {reason}");
        }
    }
    println!();
    println!("fps: encode throughput of one loop. cores: CPU seconds used per second of video,");
    println!("so the average number of busy cores in real time. PSNR: after decoding, in RGB,");
    println!("higher is better. empty: frames the encoder skipped. err 709 and err 601: largest");
    println!("channel error on flat red, green, blue and gray when a viewer assumes that color");
    println!("matrix. Browsers assume 709 for HD video that does not say. 0 means exact.");
    Ok(())
}

fn parse(args: &[String]) -> Result<(usize, usize, usize), Box<dyn Error>> {
    match args {
        [] => Ok((DEFAULT_SIZE.0, DEFAULT_SIZE.1, DEFAULT_FRAMES)),
        [width, height, frames] => Ok((width.parse()?, height.parse()?, frames.parse()?)),
        _ => Err("usage: bench_encode [width height frames]".into()),
    }
}

fn child(args: &[String]) -> Result<(), Box<dyn Error>> {
    ffmpeg::init()?;
    let (width, height, frames) = parse(args)?;
    let config = EncoderConfig::new(0, FRAME_RATE);
    let mut encoder = Encoder::with_config(width as u32, height as u32, config)?;
    let name = encoder.name();

    let distinct = (MEMORY_BUDGET / (width * height * 4)).clamp(2, MAX_DISTINCT_FRAMES);
    let sources: Vec<Frame> = (0..distinct)
        .map(|n| {
            Frame::new(
                width as u32,
                height as u32,
                source(n, width, height),
                Instant::now(),
            )
        })
        .collect();

    let mut packets = Vec::with_capacity(frames);
    let mut times = Vec::with_capacity(frames);
    let cpu_start = cpu_seconds()?;
    let wall = Instant::now();
    for n in 0..frames {
        let started = Instant::now();
        packets.push(encoder.encode(&sources[n % distinct])?);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let wall_seconds = wall.elapsed().as_secs_f64();
    let cpu_used = cpu_seconds()? - cpu_start;

    let (decoded, kept) = decode(&packets)?;
    let scores: Vec<f64> = decoded
        .iter()
        .zip(&kept)
        .step_by(PSNR_EVERY)
        .map(|(picture, &n)| {
            psnr(
                &sources[n % distinct].data,
                &to_bgra(picture, Matrix::Bt601),
            )
        })
        .collect();
    let quality = scores.iter().sum::<f64>() / scores.len().max(1) as f64;
    let empty = packets.len() - kept.len();
    let (error_709, error_601) = color_errors(width, height)?;

    let bytes: usize = packets.iter().map(Vec::len).sum();
    let video_seconds = frames as f64 / f64::from(FRAME_RATE);
    times.sort_by(f64::total_cmp);
    let p95 = times
        .get(((times.len() as f64 * 0.95) as usize).min(times.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);
    println!(
        "{name:<10} {:>5.0} {:>9.1} {:>7.1} {:>6.2} {:>7.2} {:>8.1} {empty:>6} {error_709:>8} {error_601:>8}",
        frames as f64 / wall_seconds,
        wall_seconds * 1000.0 / frames as f64,
        p95,
        cpu_used / video_seconds,
        bytes as f64 * 8.0 / video_seconds / 1e6,
        quality,
    );
    Ok(())
}

fn source(n: usize, width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0; width * height * 4];
    let scroll = n * SCROLL_PER_FRAME;
    let block_x = width / 2 + (n * 9) % (width / 2 - BLOCK_SIZE).max(1);
    let block_y = (n * 5) % (height - BLOCK_SIZE).max(1);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = if x < width / 2 {
                text_pixel(x, y + scroll)
            } else if (block_x..block_x + BLOCK_SIZE).contains(&x)
                && (block_y..block_y + BLOCK_SIZE).contains(&y)
            {
                [220, 60, 60]
            } else {
                [
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x + y + n * 4) % 256) as u8,
                ]
            };
            let at = (y * width + x) * 4;
            data[at..at + 4].copy_from_slice(&[b, g, r, 0]);
        }
    }
    data
}

fn text_pixel(x: usize, y: usize) -> [u8; 3] {
    let (cell_x, cell_y) = (x / GLYPH_WIDTH, y / GLYPH_HEIGHT);
    let (in_x, in_y) = (x % GLYPH_WIDTH, y % GLYPH_HEIGHT);
    let hash = (cell_x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((cell_y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    let hash = (hash ^ (hash >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let has_glyph = hash % 10 < 7 && in_x < 6 && in_y < 10;
    if has_glyph && (hash >> ((in_x + in_y * 6) % 40 + 8)) & 1 == 1 {
        [20, 20, 20]
    } else {
        [240, 240, 240]
    }
}

fn cpu_seconds() -> Result<f64, Box<dyn Error>> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .ok_or("unexpected /proc/self/stat")?
        .1
        .split_whitespace()
        .collect();
    let ticks: u64 = fields.get(11).ok_or("missing utime")?.parse::<u64>()?
        + fields.get(12).ok_or("missing stime")?.parse::<u64>()?;
    Ok(ticks as f64 / rustix::param::clock_ticks_per_second() as f64)
}

struct Yuv {
    width: usize,
    height: usize,
    luma: Vec<u8>,
    blue_chroma: Vec<u8>,
    red_chroma: Vec<u8>,
}

#[derive(Clone, Copy)]
enum Matrix {
    Bt601,
    Bt709,
}

fn decode(packets: &[Vec<u8>]) -> Result<(Vec<Yuv>, Vec<usize>), Box<dyn Error>> {
    let codec = decoder::find(codec::Id::H264).ok_or("no H.264 decoder in FFmpeg")?;
    let mut decoder = codec::context::Context::new_with_codec(codec)
        .decoder()
        .video()?;
    let mut pictures = Vec::new();
    let mut kept = Vec::new();
    for (n, bytes) in packets.iter().enumerate() {
        if bytes.is_empty() {
            continue;
        }
        kept.push(n);
        decoder.send_packet(&Packet::copy(bytes))?;
        receive(&mut decoder, &mut pictures)?;
    }
    decoder.send_eof()?;
    receive(&mut decoder, &mut pictures)?;
    Ok((pictures, kept))
}

fn receive(decoder: &mut decoder::Video, pictures: &mut Vec<Yuv>) -> Result<(), Box<dyn Error>> {
    let mut decoded = frame::Video::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        if !matches!(decoded.format(), Pixel::YUV420P | Pixel::YUVJ420P) {
            return Err(format!("unexpected decoded format {:?}", decoded.format()).into());
        }
        let (width, height) = (decoded.width() as usize, decoded.height() as usize);
        pictures.push(Yuv {
            width,
            height,
            luma: plane(&decoded, 0, width, height),
            blue_chroma: plane(&decoded, 1, width.div_ceil(2), height.div_ceil(2)),
            red_chroma: plane(&decoded, 2, width.div_ceil(2), height.div_ceil(2)),
        });
    }
    Ok(())
}

fn plane(decoded: &frame::Video, index: usize, width: usize, height: usize) -> Vec<u8> {
    decoded
        .data(index)
        .chunks(decoded.stride(index))
        .take(height)
        .flat_map(|line| line.iter().take(width).copied())
        .collect()
}

fn to_bgra(yuv: &Yuv, matrix: Matrix) -> Vec<u8> {
    let (red_v, green_u, green_v, blue_u) = match matrix {
        Matrix::Bt601 => (1.596, 0.392, 0.813, 2.017),
        Matrix::Bt709 => (1.793, 0.213, 0.533, 2.112),
    };
    let chroma_width = yuv.width.div_ceil(2);
    let mut data = Vec::with_capacity(yuv.width * yuv.height * 4);
    for y in 0..yuv.height {
        for x in 0..yuv.width {
            let chroma_at = (y / 2) * chroma_width + x / 2;
            let luma = 1.164 * (f32::from(yuv.luma[y * yuv.width + x]) - 16.0);
            let u = f32::from(yuv.blue_chroma[chroma_at]) - 128.0;
            let v = f32::from(yuv.red_chroma[chroma_at]) - 128.0;
            let red = luma + red_v * v;
            let green = luma - green_u * u - green_v * v;
            let blue = luma + blue_u * u;
            data.extend_from_slice(&[clamp(blue), clamp(green), clamp(red), 0]);
        }
    }
    data
}

fn clamp(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn psnr(reference: &[u8], decoded: &[u8]) -> f64 {
    let (mut sum, mut count) = (0u64, 0u64);
    for (a, b) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(decoded.as_chunks::<4>().0)
    {
        for channel in 0..3 {
            let difference = i32::from(a[channel]) - i32::from(b[channel]);
            sum += (difference * difference) as u64;
        }
        count += 3;
    }
    let mse = sum as f64 / count.max(1) as f64;
    if mse == 0.0 {
        99.0
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

fn color_errors(width: usize, height: usize) -> Result<(u8, u8), Box<dyn Error>> {
    let (mut worst_709, mut worst_601) = (0, 0);
    for [r, g, b] in FLAT_COLORS {
        let data = [b, g, r, 0].repeat(width * height);
        let flat = Frame::new(width as u32, height as u32, data, Instant::now());
        let mut encoder = Encoder::with_config(
            width as u32,
            height as u32,
            EncoderConfig::new(0, FRAME_RATE),
        )?;
        let (pictures, _) = decode(&[encoder.encode(&flat)?])?;
        let picture = pictures.first().ok_or("no picture")?;
        let center = ((height / 2) * width + width / 2) * 4;
        for (matrix, worst) in [
            (Matrix::Bt709, &mut worst_709),
            (Matrix::Bt601, &mut worst_601),
        ] {
            let pixel = to_bgra(picture, matrix);
            let got = pixel.get(center..center + 3).ok_or("short picture")?;
            for (expected, got) in [b, g, r].into_iter().zip(got) {
                *worst = (*worst).max(expected.abs_diff(*got));
            }
        }
    }
    Ok((worst_709, worst_601))
}
