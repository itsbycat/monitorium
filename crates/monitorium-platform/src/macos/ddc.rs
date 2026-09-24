use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use objc2_core_foundation::{CFAllocator, CFDictionary, CFRetained, CFString, CFType};

use super::{cf_dictionary, cf_number, symbol};

const CHIP_ADDRESS: u32 = 0x37;
const DATA_ADDRESS: u32 = 0x51;
// DDC/CI checksums start from the destination and source addresses
const REQUEST_SEED: u8 = 0x6E ^ 0x51;
const REPLY_SEED: u8 = 0x50;
const REPLY_LEN: usize = 11;
const ATTEMPTS: usize = 5;
const WRITE_CYCLES: usize = 2;
const WRITE_DELAY: Duration = Duration::from_millis(10);
const READ_DELAY: Duration = Duration::from_millis(50);
const RETRY_DELAY: Duration = Duration::from_millis(20);
// the I2C calls return before the monitor has processed the command
const COMMAND_SPACING: Duration = Duration::from_millis(50);

const SERVICE_PLANE: &CStr = c"IOService";
const ITERATE_RECURSIVELY: u32 = 1;

type IoObject = u32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IORegistryGetRootEntry(main_port: u32) -> IoObject;
    fn IORegistryEntryCreateIterator(
        entry: IoObject,
        plane: *const c_char,
        options: u32,
        iterator: *mut IoObject,
    ) -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IORegistryEntryGetName(entry: IoObject, name: *mut c_char) -> i32;
    fn IORegistryEntryGetPath(entry: IoObject, plane: *const c_char, path: *mut c_char) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: &CFString,
        allocator: Option<&CFAllocator>,
        options: u32,
    ) -> Option<NonNull<CFType>>;
}

type CreateService = unsafe extern "C" fn(*const c_void, IoObject) -> Option<NonNull<CFType>>;
type Transfer = unsafe extern "C" fn(*const c_void, u32, u32, *mut u8, u32) -> i32;

// private IOKit functions for the I2C bus of displays driven by Apple silicon
struct AvService {
    create: CreateService,
    read: Transfer,
    write: Transfer,
}

fn api() -> Option<&'static AvService> {
    static API: OnceLock<Option<AvService>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        Some(AvService {
            create: symbol(libc::RTLD_DEFAULT, c"IOAVServiceCreateWithService")?,
            read: symbol(libc::RTLD_DEFAULT, c"IOAVServiceReadI2C")?,
            write: symbol(libc::RTLD_DEFAULT, c"IOAVServiceWriteI2C")?,
        })
    })
    .as_ref()
}

struct Entry(IoObject);

impl Drop for Entry {
    fn drop(&mut self) {
        unsafe {
            IOObjectRelease(self.0);
        }
    }
}

/// An external display's DDC/CI service, not opened yet.
pub struct Service {
    /// IORegistry path of the framebuffer, as in CoreDisplay's `IODisplayLocation`
    location: String,
    vendor: Option<u32>,
    model: Option<u32>,
    serial: Option<u32>,
    entry: Entry,
}

impl Service {
    pub fn open(self) -> Option<Channel> {
        let service = unsafe { (api()?.create)(std::ptr::null(), self.entry.0) }?;
        Some(Channel {
            service: unsafe { CFRetained::from_raw(service) },
            last_io: None,
        })
    }
}

pub fn services() -> Vec<Service> {
    if api().is_none() {
        return Vec::new();
    }
    let mut iterator = 0;
    let root = Entry(unsafe { IORegistryGetRootEntry(0) });
    let status = unsafe {
        IORegistryEntryCreateIterator(
            root.0,
            SERVICE_PLANE.as_ptr(),
            ITERATE_RECURSIVELY,
            &mut iterator,
        )
    };
    if status != 0 {
        log::warn!("iterating the IORegistry failed ({status:#x})");
        return Vec::new();
    }
    let iterator = Entry(iterator);

    // opening a service changes the registry and ends the iteration, so only collect here
    let mut framebuffer: Option<(String, Option<CFRetained<CFDictionary>>)> = None;
    let mut services = Vec::new();
    loop {
        let entry = unsafe { IOIteratorNext(iterator.0) };
        if entry == 0 {
            break;
        }
        let entry = Entry(entry);
        match name(&entry).as_str() {
            "AppleCLCD2" | "IOMobileFramebufferShim" => {
                let attributes = property(&entry, "DisplayAttributes")
                    .and_then(|a| a.downcast::<CFDictionary>().ok())
                    .and_then(|a| cf_dictionary(&a, "ProductAttributes"));
                framebuffer = Some((path(&entry), attributes));
            }
            // an external display's service follows its framebuffer in the registry
            "DCPAVServiceProxy" => {
                let external = property(&entry, "Location")
                    .and_then(|l| l.downcast::<CFString>().ok())
                    .is_some_and(|l| l.to_string() == "External");
                if external && let Some((location, product)) = framebuffer.take() {
                    let number = |key| {
                        product
                            .as_deref()
                            .and_then(|p| cf_number(p, key))
                            .and_then(|n| u32::try_from(n).ok())
                    };
                    services.push(Service {
                        vendor: number("LegacyManufacturerID"),
                        model: number("ProductID"),
                        serial: number("SerialNumber"),
                        location,
                        entry,
                    });
                }
            }
            _ => {}
        }
    }
    services
}

/// Takes the service for a display: the one on its framebuffer, or else the only one with its
/// vendor, model and serial number.
pub fn take(
    services: &mut Vec<Service>,
    location: Option<&str>,
    (vendor, model, serial): (u32, u32, u32),
) -> Option<Service> {
    let index = location
        .and_then(|location| services.iter().position(|s| s.location == location))
        .or_else(|| {
            let mut matching = services.iter().enumerate().filter(|(_, s)| {
                s.vendor == Some(vendor) && s.model == Some(model) && s.serial == Some(serial)
            });
            match (matching.next(), matching.next()) {
                (Some((index, _)), None) => Some(index),
                _ => None,
            }
        })?;
    Some(services.swap_remove(index))
}

pub struct Channel {
    service: CFRetained<CFType>,
    last_io: Option<Instant>,
}

impl Channel {
    /// Reads a VCP feature as (current, max).
    pub fn read(&mut self, code: u8) -> Option<(u16, u16)> {
        let api = api()?;
        let request = get_request(code);
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                thread::sleep(RETRY_DELAY);
            }
            self.send(api, &request);
            thread::sleep(READ_DELAY);
            let mut reply = [0u8; REPLY_LEN];
            let status = unsafe {
                (api.read)(
                    self.ptr(),
                    CHIP_ADDRESS,
                    0,
                    reply.as_mut_ptr(),
                    REPLY_LEN as u32,
                )
            };
            self.last_io = Some(Instant::now());
            if status == 0
                && let Some(value) = parse_reply(&reply, code)
            {
                return Some(value);
            }
        }
        None
    }

    pub fn write(&mut self, code: u8, value: u16) -> bool {
        let Some(api) = api() else {
            return false;
        };
        let request = set_request(code, value);
        (0..ATTEMPTS).any(|attempt| {
            if attempt > 0 {
                thread::sleep(RETRY_DELAY);
            }
            self.send(api, &request)
        })
    }

    // monitors miss a command now and then, so every command goes out twice
    fn send(&mut self, api: &AvService, packet: &[u8]) -> bool {
        if let Some(wait) = self
            .last_io
            .and_then(|t| COMMAND_SPACING.checked_sub(t.elapsed()))
        {
            thread::sleep(wait);
        }
        let mut sent = false;
        for _ in 0..WRITE_CYCLES {
            thread::sleep(WRITE_DELAY);
            let mut buffer = packet.to_vec();
            let status = unsafe {
                (api.write)(
                    self.ptr(),
                    CHIP_ADDRESS,
                    DATA_ADDRESS,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                )
            };
            sent |= status == 0;
        }
        self.last_io = Some(Instant::now());
        sent
    }

    fn ptr(&self) -> *const c_void {
        CFRetained::as_ptr(&self.service).as_ptr().cast()
    }
}

// length byte (0x80 | body length), body, checksum
fn get_request(code: u8) -> [u8; 4] {
    with_checksum([0x82, 0x01, code, 0])
}

fn set_request(code: u8, value: u16) -> [u8; 6] {
    let [high, low] = value.to_be_bytes();
    with_checksum([0x84, 0x03, code, high, low, 0])
}

fn with_checksum<const N: usize>(mut packet: [u8; N]) -> [u8; N] {
    packet[N - 1] = checksum(REQUEST_SEED, &packet[..N - 1]);
    packet
}

fn checksum(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |sum, byte| sum ^ byte)
}

/// Parses a Get VCP Feature reply into (current, max).
fn parse_reply(reply: &[u8; REPLY_LEN], code: u8) -> Option<(u16, u16)> {
    // source, length, opcode, result, code, type, max (2), current (2), checksum
    let valid = reply[0] == 0x6E
        && reply[1] == 0x88
        && reply[2] == 0x02
        && reply[3] == 0
        && reply[4] == code
        && checksum(REPLY_SEED, &reply[..REPLY_LEN - 1]) == reply[REPLY_LEN - 1];
    let max = u16::from_be_bytes([reply[6], reply[7]]);
    let current = u16::from_be_bytes([reply[8], reply[9]]);
    (valid && max > 0).then_some((current, max))
}

fn name(entry: &Entry) -> String {
    let mut buffer = [0 as c_char; 128];
    if unsafe { IORegistryEntryGetName(entry.0, buffer.as_mut_ptr()) } != 0 {
        return String::new();
    }
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn path(entry: &Entry) -> String {
    let mut buffer = [0 as c_char; 512];
    if unsafe { IORegistryEntryGetPath(entry.0, SERVICE_PLANE.as_ptr(), buffer.as_mut_ptr()) } != 0
    {
        return String::new();
    }
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn property(entry: &Entry, key: &str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_str(key);
    let value = unsafe { IORegistryEntryCreateCFProperty(entry.0, &key, None, 0) }?;
    Some(unsafe { CFRetained::from_raw(value) })
}

#[cfg(test)]
mod tests {
    use super::*;

    // a real reply from an ASUS VG248 at 100 of 100
    const REPLY: [u8; REPLY_LEN] = [
        0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x64, 0xA4,
    ];

    #[test]
    fn requests() {
        assert_eq!(get_request(0x10), [0x82, 0x01, 0x10, 0xAC]);
        assert_eq!(set_request(0x10, 70), [0x84, 0x03, 0x10, 0x00, 0x46, 0xEE]);
        assert_eq!(set_request(0xD6, 4), [0x84, 0x03, 0xD6, 0x00, 0x04, 0x6A]);
    }

    #[test]
    fn replies() {
        assert_eq!(parse_reply(&REPLY, 0x10), Some((100, 100)));
        assert_eq!(parse_reply(&REPLY, 0x12), None);

        let mut corrupted = REPLY;
        corrupted[9] = 0x63;
        assert_eq!(parse_reply(&corrupted, 0x10), None);

        // "unsupported VCP code", with a matching checksum
        let mut unsupported = REPLY;
        unsupported[3] = 0x01;
        unsupported[10] ^= 0x01;
        assert_eq!(parse_reply(&unsupported, 0x10), None);
    }
}
