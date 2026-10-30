//! A screenshot of a game's window taken by Windows itself (PrintWindow), for runs that stall: F2 needs a game that
//! still runs its render loop, and a frozen game or an error screen ("Out of memory!") never takes one. A minimized
//! window is shown for a moment without taking focus (Windows draws nothing for it while minimized), then put back.

#[repr(C)]
#[derive(Default)]
struct Rect { l: i32, t: i32, r: i32, b: i32 }

#[repr(C)]
struct BitmapInfo {
    size: u32, width: i32, height: i32, planes: u16, bits: u16, compression: u32, image_size: u32,
    xppm: i32, yppm: i32, used: u32, important: u32, colors: [u32; 1],
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetWindowRect(h: isize, r: *mut Rect) -> i32;
    fn GetDC(h: isize) -> isize;
    fn ReleaseDC(h: isize, dc: isize) -> i32;
    fn PrintWindow(h: isize, dc: isize, flags: u32) -> i32;
    fn IsIconic(h: isize) -> i32;
    fn ShowWindow(h: isize, cmd: i32) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
    fn SelectObject(dc: isize, o: isize) -> isize;
    fn DeleteObject(o: isize) -> i32;
    fn DeleteDC(dc: isize) -> i32;
    fn GetDIBits(dc: isize, bmp: isize, start: u32, lines: u32, bits: *mut u8, info: *mut BitmapInfo, usage: u32) -> i32;
}

/// PrintWindow's PW_RENDERFULLCONTENT: asks DWM for what the window shows, which also works for OpenGL games.
const PW_RENDERFULLCONTENT: u32 = 2;

/// The window's pixels as RGBA, top row first.
fn grab(h: isize) -> Option<(u32, u32, Vec<u8>)> {
    let mut r = Rect::default();
    if unsafe { GetWindowRect(h, &mut r) } == 0 {
        return None;
    }
    let (w, ht) = (r.r - r.l, r.b - r.t);
    if w < 50 || ht < 50 {
        return None;
    }
    unsafe {
        let screen = GetDC(0);
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, ht);
        let old = SelectObject(dc, bmp);
        let ok = PrintWindow(h, dc, PW_RENDERFULLCONTENT) != 0;
        let mut px = vec![0u8; (w * ht * 4) as usize];
        let mut info = BitmapInfo { size: 40, width: w, height: -ht, planes: 1, bits: 32, compression: 0, image_size: 0,
            xppm: 0, yppm: 0, used: 0, important: 0, colors: [0] };
        SelectObject(dc, old);
        let lines = if ok { GetDIBits(dc, bmp, 0, ht as u32, px.as_mut_ptr(), &mut info, 0) } else { 0 };
        DeleteObject(bmp);
        DeleteDC(dc);
        ReleaseDC(0, screen);
        if lines <= 0 {
            return None;
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        Some((w as u32, ht as u32, px))
    }
}

/// Save a screenshot of the window to `out` (PNG). Err says why there is none; a picture that came out all black
/// (a window Windows had nothing drawn for) is still saved, and said so.
pub fn window_png(h: isize, out: &std::path::Path) -> Result<String, String> {
    let minimized = unsafe { IsIconic(h) } != 0;
    if minimized {
        unsafe { ShowWindow(h, 4) }; // SW_SHOWNOACTIVATE: drawn again, the app in front stays in front
        std::thread::sleep(std::time::Duration::from_millis(600));
    }
    let got = grab(h);
    if minimized {
        unsafe { ShowWindow(h, 7) }; // SW_SHOWMINNOACTIVE
    }
    let (w, ht, px) = got.ok_or("Windows couldn't capture the game's window")?;
    let black = px.chunks_exact(4).all(|p| p[0] < 8 && p[1] < 8 && p[2] < 8);
    let f = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, ht);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&px)).map_err(|e| e.to_string())?;
    Ok(if black { "all black: the window drew nothing Windows could capture".into() } else { String::new() })
}

