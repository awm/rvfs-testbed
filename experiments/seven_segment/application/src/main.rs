#![doc = include_str!("../../README.md")]
#![no_main]
#![no_std]
#![warn(missing_docs)]

use bsp::{entry, hal, hal::dma::DMAExt, hal::fugit::RateExtU32, hal::Clock, pac};
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi};
use hexchain::{Display, DisplayData};
use panic_halt as _;
use rp_pico as bsp;

mod hexchain;

/// Application entry point.
#[entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        bsp::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();
    let mut timer = hal::timer::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);

    let sio = hal::Sio::new(pac.SIO);
    let pins = bsp::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let dma = pac.DMA.split(&mut pac.RESETS);

    // Onboard LED
    let mut led_pin = pins.led.into_push_pull_output();

    // SPI 1 configuration
    let sclk = pins.gpio10.into_function::<hal::gpio::FunctionSpi>();
    let mosi = pins.gpio11.into_function::<hal::gpio::FunctionSpi>();
    let cs = pins.gpio13.into_push_pull_output();
    let spi_pins = (mosi, sclk);
    let spi = hal::spi::Spi::<_, _, _>::new(pac.SPI1, spi_pins).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        64_000_000.Hz(),
        spi::MODE_0,
    );

    let mut display = Display::new(spi, cs, dma.ch0, sio.interp0);

    // Blink all of the decimal points three times on startup
    let mut show = true;
    let mut decimals = [true; hexchain::CHAIN_LENGTH];
    display.set(DisplayData::AllOff);
    for _ in 0..6 {
        led_pin.set_state(show.into()).unwrap();
        display.set(DisplayData::DecimalPoints(&decimals));
        display.show();
        timer.delay_ms(500);

        show = !show;
        decimals.fill(show);
    }
    decimals.fill(false);
    display.set(DisplayData::DecimalPoints(&decimals));
    display.set(DisplayData::AllOn);

    // Start displaying rolling counter
    let mut bytes: [u8; hexchain::DATA_LENGTH] = [0; hexchain::DATA_LENGTH];
    let index = bytes.len() - 1;
    loop {
        led_pin.set_high().unwrap();
        timer.delay_ms(500);
        led_pin.set_low().unwrap();
        timer.delay_ms(500);

        display.set(DisplayData::Values(&bytes));
        display.show();

        bytes[index] = bytes[index].wrapping_add(1);
    }
}
