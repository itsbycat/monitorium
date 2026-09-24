use eframe::egui;

const TRAY_WHITE: &[u8] = include_bytes!("../assets/icons/tray-white.png");
const TRAY_BLACK: &[u8] = include_bytes!("../assets/icons/tray-black.png");
const APP_ICON: &[u8] = include_bytes!("../assets/icons/128x128.png");

fn decode(bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let image = image::load_from_memory(bytes)
        .expect("embedded icon is a valid PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    (image.into_raw(), width, height)
}

pub fn tray_icon(light_taskbar: bool) -> tray_icon::Icon {
    let (rgba, width, height) = decode(if light_taskbar {
        TRAY_BLACK
    } else {
        TRAY_WHITE
    });
    tray_icon::Icon::from_rgba(rgba, width, height).expect("valid tray icon")
}

pub fn window_icon() -> egui::IconData {
    let (rgba, width, height) = decode(APP_ICON);
    egui::IconData {
        rgba,
        width,
        height,
    }
}

pub fn logo_image(dark_mode: bool) -> egui::ColorImage {
    let (rgba, width, height) = decode(if dark_mode { TRAY_WHITE } else { TRAY_BLACK });
    egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba)
}
