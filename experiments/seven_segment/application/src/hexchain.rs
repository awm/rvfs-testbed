//! Experimental driver for a sequence of shift register linked seven segment displays.
//!
//! ## Display Segment Organization
//!
//! ```
//!     A
//!    ---
//! F | G | B
//!    ---
//! E |   | C
//!    --- o
//!     D  DP
//! ```
//!

use cortex_m::singleton;
use embedded_hal::{digital::OutputPin, spi::SpiBus};
use rp_pico::hal::{dma, sio, sio::Lane};

/// Maximum display chain length (number of hex digits).
pub const CHAIN_LENGTH: usize = u32::BITS as usize;
/// Maximum size of buffer that can be displayed (each nibble corresponds to one display digit).
pub const DATA_LENGTH: usize = CHAIN_LENGTH / 2;

/// Bit corresponding to display segment "A."
const SEG_A: u8 = 1 << 0;
/// Bit corresponding to display segment "B."
const SEG_B: u8 = 1 << 1;
/// Bit corresponding to display segment "C."
const SEG_C: u8 = 1 << 2;
/// Bit corresponding to display segment "D."
const SEG_D: u8 = 1 << 3;
/// Bit corresponding to display segment "E."
const SEG_E: u8 = 1 << 4;
/// Bit corresponding to display segment "F."
const SEG_F: u8 = 1 << 5;
/// Bit corresponding to display segment "G."
const SEG_G: u8 = 1 << 6;
/// Bit corresponding to decimal point.
const SEG_DP: u8 = 1 << 7;

/// Lookup table constructing each hexadecimal character using the most appropriate display segments.
const CHAR_TABLE: [u8; 16] = [
    !(SEG_G | SEG_DP),                 // 0x0
    SEG_B | SEG_C,                     // 0x1
    !(SEG_C | SEG_F | SEG_DP),         // 0x2
    !(SEG_E | SEG_F | SEG_DP),         // 0x3
    !(SEG_A | SEG_D | SEG_E | SEG_DP), // 0x4
    !(SEG_B | SEG_E | SEG_DP),         // 0x5
    !(SEG_B | SEG_DP),                 // 0x6
    SEG_A | SEG_B | SEG_C,             // 0x7
    !SEG_DP,                           // 0x8
    !(SEG_E | SEG_DP),                 // 0x9
    !(SEG_D | SEG_DP),                 // 0xA
    !(SEG_A | SEG_B | SEG_DP),         // 0xb
    SEG_A | SEG_F | SEG_E | SEG_D,     // 0xC
    !(SEG_A | SEG_F | SEG_DP),         // 0xd
    !(SEG_B | SEG_C | SEG_DP),         // 0xE
    !(SEG_B | SEG_C | SEG_D | SEG_DP), // 0xF
];

/// RP2040 interpolator wrapper which configures and runs it as a character lookup table index generator.
pub trait Interp {
    /// Configure interpolator for lookup index generation.
    fn init(&mut self);
    /// Generate a pair of (high nibble, low nibble) character lookup table indices for the given byte of data.
    fn run(&mut self, data: u8) -> (usize, usize);
}

/// Interpolator handling of lookup generation for each interpolator instance.
macro_rules! interpolators {
    ( $($interp:ident),+ ) => {
            $(
                impl Interp for sio::$interp {
                    fn init(&mut self) {
                        self.get_lane0().set_base(0);
                        self.get_lane1().set_base(0);

                        let mut lanectrl = sio::LaneCtrl::new();
                        lanectrl.mask_msb = 3;
                        self.get_lane0().set_ctrl(lanectrl.encode());

                        lanectrl.cross_input = true;
                        lanectrl.shift = 4;
                        self.get_lane1().set_ctrl(lanectrl.encode());
                    }
                    fn run(&mut self, data: u8) -> (usize, usize) {
                        self.get_lane0().set_accum(data as u32);
                        let lo = self.get_lane0().peek() as usize;
                        let hi = self.get_lane1().peek() as usize;
                        (hi, lo)
                    }
                }
            )+
        };
}

interpolators!(Interp0, Interp1);

/// Data which modifies the display state.
#[allow(dead_code)]
pub enum DisplayData<'a> {
    /// Turn on all digits (decimal points are unaffected).
    AllOn,
    /// Turn off (blank) all digits (decimal points are unaffected).
    AllOff,
    /// Turn on those digits which are marked `true`, blank all others (decimal points are unaffected).
    On(&'a [bool; CHAIN_LENGTH]),
    /// Set the decimal point for all digits which are marked `true`, clear it for all others.
    DecimalPoints(&'a [bool; CHAIN_LENGTH]),
    /// Provide the data to display for all digits.
    Values(&'a [u8; DATA_LENGTH]),
}

/// Display chain controller.
pub struct Display<S, C, D, I>
where
    S: dma::WriteTarget<TransmittedWord = u8> + SpiBus,
    C: OutputPin,
    D: dma::SingleChannel,
    I: Interp,
{
    /// SPI port from which to shift data.
    spi: Option<S>,
    /// Chip select line for SPI port.
    cs: C,
    /// DMA channel to use for bitstream transmission.
    dma: Option<D>,
    /// Interpolator to use for character lookup index generation.
    interp: I,

    /// On state of each display chain member.
    on: [bool; CHAIN_LENGTH],
    /// Decimal point state of each display chain member.
    points: [u8; CHAIN_LENGTH],
    /// Data to be displayed as hexadecimal.
    data: [u8; DATA_LENGTH],
    /// Bitstream to be shifted out to the display members.
    bits: Option<&'static mut [u8; CHAIN_LENGTH]>,
}

impl<S, C, D, I> Display<S, C, D, I>
where
    S: dma::WriteTarget<TransmittedWord = u8> + SpiBus,
    C: OutputPin,
    D: dma::SingleChannel,
    I: Interp,
{
    /// Instantiate a display controller.
    ///
    /// **Arguments:**
    ///
    /// * `spi`:    SPI port instance for transmitting display bitstream.
    /// * `cs`:     Output pin for SPI chip select/shift register latch.
    /// * `dma`:    DMA channel for SPI transmission.
    /// * `interp`:  Interpolator instance to use for character lookup calculations.
    pub fn new(spi: S, mut cs: C, dma: D, mut interp: I) -> Self {
        interp.init();
        let _ = cs.set_high();

        Self {
            spi: Some(spi),
            cs,
            dma: Some(dma),
            interp,
            on: [false; CHAIN_LENGTH],
            points: [0; CHAIN_LENGTH],
            data: [0; DATA_LENGTH],
            bits: singleton!(: [u8; CHAIN_LENGTH] = [0; CHAIN_LENGTH]),
        }
    }

    /// Set or modify the display representation.
    ///
    /// This updates aspects of the driver's internal representation of the display.  The changes will not be applied to
    /// the physical display until show is called.
    ///
    /// **Arguments:**
    ///
    /// * `data`: New display properties to set.
    pub fn set(&mut self, data: DisplayData) {
        match data {
            DisplayData::AllOn => {
                self.on.fill(true);
            }
            DisplayData::AllOff => {
                self.on.fill(false);
            }
            DisplayData::On(symbols) => {
                self.on = *symbols;
            }
            DisplayData::DecimalPoints(symbols) => {
                for i in 0..CHAIN_LENGTH {
                    self.points[i] = if symbols[i] { SEG_DP } else { 0x00 };
                }
            }
            DisplayData::Values(bytes) => {
                self.data = *bytes;
            }
        }
    }

    /// Update one byte of the bitstream based on the internal state for that display element.
    ///
    /// **Arguments:**
    ///
    /// * `bits`:           Bitstream array to render into.
    /// * `chain_index`:    Index of the byte to update in the bitstream.
    /// * `char_lookup`:    Character table lookup index for the data representation.
    fn render_bits(
        &mut self,
        bits: &mut [u8; CHAIN_LENGTH],
        chain_index: usize,
        char_lookup: usize,
    ) {
        bits[chain_index] = self.points[chain_index];
        if self.on[chain_index] {
            bits[chain_index] |= CHAR_TABLE[char_lookup];
        }
    }

    /// Apply the internal driver state to the physical display chain.
    pub fn show(&mut self) {
        let dma = self.dma.take().unwrap();
        let bits = self.bits.take().unwrap();
        let spi = self.spi.take().unwrap();

        for i in 0..DATA_LENGTH {
            let (hi, lo) = self.interp.run(self.data[i]);

            let chain_index = i * 2;
            self.render_bits(bits, chain_index, lo);
            self.render_bits(bits, chain_index + 1, hi);
        }

        let _ = self.cs.set_low();
        let transfer = dma::single_buffer::Config::new(dma, bits, spi).start();
        let (dma, bits, mut spi) = transfer.wait();
        // Also need to wait for the SPI transmission to complete.
        spi.flush().unwrap();
        let _ = self.cs.set_high();

        self.dma = Some(dma);
        self.bits = Some(bits);
        self.spi = Some(spi);
    }
}
