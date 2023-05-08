//! Blinks the LED on a Pico board
//!
//! This will blink an LED attached to GP2
#![no_std]
#![no_main]

extern crate cortex_m_rt;

use bsp::entry;
use core::hint::spin_loop;
use defmt::*;
use defmt_rtt as _;
use panic_probe as _;

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
    pio::PIOExt,
    sio::Sio,
    spi,
    watchdog::Watchdog,
};
use embedded_hal::digital::v2::OutputPin;

// Link in the embedded_sdmmc crate.
// The `SdMmcSpi` is used for block level access to the card.
// And the `Controller` gives access to the FAT filesystem functions.
use embedded_sdmmc::{Controller, SdMmcSpi, TimeSource, Timestamp, VolumeIdx};

// Get the file open mode enum:
use embedded_sdmmc::filesystem::Mode;

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
    let mut delay = cortex_m::delay::Delay::new(core.SYST, clocks.system_clock.freq().to_Hz());
    info!("Aquire SPI SD/MMC BlockDevice...");
    let mut sdspi = SdMmcSpi::new(spi, spi_cs);

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

    // This shows how to iterate through the directory and how
    // to get the file names (and print them in hope they are UTF-8 compatible):
    cont.iterate_dir(&volume, &dir, |ent| {
        info!(
            "/{}.{}",
            core::str::from_utf8(ent.name.base_name()).unwrap(),
            core::str::from_utf8(ent.name.extension()).unwrap()
        );
    })
    .unwrap();

    // Next we going to read a file from the SD card:
    if let Ok(mut file) = cont.open_file_in_dir(&mut volume, &dir, "test2.txt", Mode::ReadOnly) {
        let mut buf = [0u8; 32];
        let read_count = cont.read(&volume, &mut file, &mut buf).unwrap();
        cont.close_file(&volume, file).unwrap();

        if read_count >= 2 {
            info!("READ {} bytes: {}", read_count, buf);
        }
    }
    loop {}
}
