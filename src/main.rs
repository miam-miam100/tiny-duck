//! Blinks the LED on a Pico board
//!
//! This will blink an LED attached to GP2
#![no_std]
#![no_main]

extern crate cortex_m_rt;

use bsp::entry;

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
mod commands;
use commands::Command;

// Provide an alias for our BSP so we can switch targets quickly.
// Uncomment the BSP you included in Cargo.toml, the rest of the code does not need to change.
use rp2040_hal as bsp;
// use sparkfun_pro_micro_rp2040 as bsp;
use fugit::RateExtU32;

use bsp::{
    clocks::{init_clocks_and_plls, Clock},
    gpio,
    gpio::Pins,
    pac,
    sio::Sio,
    spi,
    watchdog::Watchdog,
};
use cortex_m::delay::Delay;
use embedded_hal::digital::v2::OutputPin;

// Link in the embedded_sdmmc crate.
// The `SdMmcSpi` is used for block level access to the card.
// And the `Controller` gives access to the FAT filesystem functions.
use embedded_sdmmc::{Controller, SdMmcSpi, TimeSource, Timestamp, VolumeIdx};

// Get the file open mode enum:
use embedded_sdmmc::filesystem::Mode;
use prse::parse;
use rp2040_hal::pac::interrupt;

/// A dummy time source, which is mostly important for creating files.
#[derive(Default)]
pub struct DummyTimeSource;

impl TimeSource for DummyTimeSource {
    // In theory you could use the RTC of the rp2040 here, if you had
    // any external time synchronizing device.
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

#[link_section = ".boot2"]
#[no_mangle]
#[used]
pub static BOOT2_FIRMWARE: [u8; 256] = rp2040_boot2::BOOT_LOADER_W25Q080;

// USB Device support
use usb_device::{class_prelude::*, prelude::*};

// USB Human Interface Device (HID) Class support
use usbd_hid::descriptor::generator_prelude::*;
use usbd_hid::descriptor::{KeyboardReport, MouseReport};
use usbd_hid::hid_class::HIDClass;

/// The USB Device Driver (shared with the interrupt).
static mut USB_DEVICE: Option<UsbDevice<bsp::usb::UsbBus>> = None;

/// The USB Bus Driver (shared with the interrupt).
static mut USB_BUS: Option<UsbBusAllocator<bsp::usb::UsbBus>> = None;

/// The USB Human Interface Device Mouse (shared with the interrupt).
static mut USB_MOUSE: Option<HIDClass<bsp::usb::UsbBus>> = None;

const POLL_RATE: u8 = 10;
const TICK_RATE: u32 = 2 * POLL_RATE as u32;

#[entry]
fn main() -> ! {
    info!("Program start");
    let mut pac = pac::Peripherals::take().unwrap();
    let core = pac::CorePeripherals::take().unwrap();
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    let sio = Sio::new(pac.SIO);

    // External high-speed crystal on the pico board is 12Mhz
    let external_xtal_freq_hz = 12_000_000u32;
    let clocks = init_clocks_and_plls(
        external_xtal_freq_hz,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let pins = Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // Set up the USB driver
    let usb_bus = UsbBusAllocator::new(bsp::usb::UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));

    unsafe {
        // Note (safety): This is safe as interrupts haven't been started yet
        USB_BUS = Some(usb_bus);
    }

    // Grab a reference to the USB Bus allocator. We are promising to the
    // compiler not to take mutable access to this global variable whilst this
    // reference exists!
    let bus_ref = unsafe { USB_BUS.as_ref().unwrap() };

    // Set up the USB HID Class Device driver, providing Mouse Reports
    let usb_mouse = HIDClass::new(bus_ref, MouseReport::desc(), POLL_RATE);

    unsafe {
        // Note (safety): This is safe as interrupts haven't been started yet.
        USB_MOUSE = Some(usb_mouse);
    }

    // Create a USB device with a fake VID and PID
    let usb_dev = UsbDeviceBuilder::new(bus_ref, UsbVidPid(0x16c0, 0x27da))
        .manufacturer("Miam Inc")
        .product("Tiny Duck")
        .serial_number("TEST")
        .composite_with_iads()
        .build();

    unsafe {
        // Note (safety): This is safe as interrupts haven't been started yet
        USB_DEVICE = Some(usb_dev);
    }

    unsafe {
        // Enable the USB interrupt
        pac::NVIC::unmask(pac::Interrupt::USBCTRL_IRQ);
    };

    // These are implicitly used by the spi driver if they are in the correct mode
    let spi_sclk = pins.gpio10.into::<gpio::FunctionSpi, gpio::PullNone>();
    let spi_mosi = pins.gpio11.into::<gpio::FunctionSpi, gpio::PullNone>();
    let spi_miso = pins.gpio12.into::<gpio::FunctionSpi, gpio::PullNone>();
    pins.gpio13.into_pull_up_disabled();
    pins.gpio14.into_pull_up_disabled();

    // Create an SPI driver instance for the SPI0 device
    let spi = spi::Spi::<_, _, _, 8>::new(pac.SPI1, (spi_mosi, spi_miso, spi_sclk));

    // Exchange the uninitialised SPI driver for an initialised one
    let spi = spi.init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        50_000u32.Hz(),
        &embedded_hal::spi::MODE_0,
    );

    let spi_cs = pins.gpio15.into_push_pull_output();
    let mut delay = Delay::new(core.SYST, clocks.system_clock.freq().to_Hz());
    delay.delay_ms(1000);

    // Having problems with the SD Card stuff so we will just embed the file
    // get_file(SdMmcSpi::new(spi, spi_cs));
    get_commands(include_bytes!("../example.td"), true, &mut delay);
    loop {}
}

fn get_file<SPI, CS>(mut sdspi: SdMmcSpi<SPI, CS>, delay: &mut Delay) -> !
where
    SPI: embedded_hal::blocking::spi::Transfer<u8>,
    CS: OutputPin,
    <SPI as embedded_hal::blocking::spi::Transfer<u8>>::Error: core::fmt::Debug,
{
    info!("Aquire SPI SD/MMC BlockDevice...");
    // Next we need to acquire the block device and initialize the
    // communication with the SD card.
    let block = match sdspi.acquire() {
        Ok(block) => block,
        Err(e) => {
            error!("Error retrieving card size: {}", defmt::Debug2Format(&e));
            loop {}
        }
    };

    info!("Init SD card controller...");
    let mut cont = Controller::new(block, DummyTimeSource::default());

    info!("OK!\nCard size...");
    match cont.device().card_size_bytes() {
        Ok(size) => info!("card size is {} bytes", size),
        Err(e) => {
            error!("Error retrieving card size: {}", defmt::Debug2Format(&e));
        }
    }

    info!("Getting Volume 0...");
    let mut volume = match cont.get_volume(VolumeIdx(0)) {
        Ok(v) => v,
        Err(e) => {
            error!("Error getting volume 0: {}", defmt::Debug2Format(&e));
            loop {}
        }
    };

    // After we have the volume (partition) of the drive we got to open the
    // root directory:
    let dir = match cont.open_root_dir(&volume) {
        Ok(dir) => dir,
        Err(e) => {
            error!("Error opening root dir: {}", defmt::Debug2Format(&e));
            loop {}
        }
    };

    info!("Root directory opened!");

    // Next we going to read a file from the SD card:
    if let Ok(mut file) = cont.open_file_in_dir(&mut volume, &dir, "main.td", Mode::ReadOnly) {
        let mut buf = [0u8; 64];
        while !file.eof() {
            let read_count = cont.read(&volume, &mut file, &mut buf).unwrap();
            let seek_to = get_commands(&buf[..read_count], file.eof(), delay);
            if let Some(seek) = seek_to {
                file.seek_from_current(seek).unwrap();
            }
            buf = [0; 64];
        }
        cont.close_file(&volume, file).unwrap();
    }
    loop {}
}

/// Returns the index up to which it has parsed.
fn get_commands(buf: &[u8], read_full: bool, delay: &mut Delay) -> Option<i32> {
    let mut result = None;
    let buf = if read_full {
        buf
    } else {
        let mut idx = buf
            .iter()
            .enumerate()
            .rfind(|(_, &c)| c == b'\n')
            .unwrap()
            .0;

        let diff = idx as i32 - (buf.len() as i32 - 1);
        result = Some(diff);
        if buf.get(idx - 1) == Some(&(b'\r')) {
            idx -= 1;
        }
        &buf[..idx]
    };
    for com in core::str::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|l| parse!(l, "{}"))
    {
        let com: Command = com;
        com.run(delay);
    }
    result
}

/// This function is called whenever the USB Hardware generates an Interrupt
/// Request.
#[allow(non_snake_case)]
#[interrupt]
unsafe fn USBCTRL_IRQ() {
    // Handle USB request
    info!("Handle usb request.");
    let usb_dev = USB_DEVICE.as_mut().unwrap();
    let usb_mouse = USB_MOUSE.as_mut().unwrap();
    usb_dev.poll(&mut [usb_mouse]);
}
