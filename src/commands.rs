use crate::{POLL_RATE, TICK_RATE};
use cortex_m::delay::Delay;
use defmt::*;
use prse::Parse;
use usbd_hid::descriptor::{KeyboardReport, MouseReport};
use usbd_hid::hid_class::HidProtocol::Keyboard;

#[derive(Parse, Debug, PartialEq, Copy, Clone)]
pub enum Command {
    #[prse = "wait {}"]
    Wait(u32),
    #[prse = "move {x} {y}"]
    MoveMouse { x: i8, y: i8 },
    #[prse = "click"]
    Click,
}

impl Command {
    pub fn run(&self, delay: &mut Delay) {
        match *self {
            Command::Wait(t) => {
                info!("Found wait {}", t);
                delay.delay_ms(t * TICK_RATE);
                info!("Finished waiting");
            }
            Command::MoveMouse { x, y } => {
                info!("Found mouse move with x = {}, y = {}", x, y);
                let report = MouseReport {
                    buttons: 0,
                    x,
                    y,
                    wheel: 0,
                    pan: 0,
                };
                critical_section::with(|_| unsafe {
                    // Now interrupts are disabled, grab the global variable and, if
                    // available, send it a HID report
                    crate::USB_MOUSE.as_mut().map(|hid| hid.push_input(&report))
                })
                .unwrap();
                delay.delay_ms(TICK_RATE);
            }
            Command::Click => {
                info!("Found click");
                let report1 = MouseReport {
                    buttons: 1,
                    x: 0,
                    y: 0,
                    wheel: 0,
                    pan: 0,
                };
                let report2 = MouseReport {
                    buttons: 0,
                    ..report1
                };
                critical_section::with(|_| unsafe {
                    // Now interrupts are disabled, grab the global variable and, if
                    // available, send it a HID report
                    crate::USB_MOUSE
                        .as_mut()
                        .map(|hid| hid.push_input(&report1))
                })
                .unwrap();
                delay.delay_ms(TICK_RATE);
                critical_section::with(|_| unsafe {
                    // Now interrupts are disabled, grab the global variable and, if
                    // available, send it a HID report
                    crate::USB_MOUSE
                        .as_mut()
                        .map(|hid| hid.push_input(&report2))
                })
                .unwrap();
            }
        }
    }
}
