#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use embassy_executor::Spawner;
use embassy_sync::signal::Signal;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_hal::clock::CpuClock;
use esp_hal::Async;
use esp_hal::Blocking;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::delay::{Delay};
use esp_hal::analog::adc::{Adc, AdcPin, AdcConfig, Attenuation, AdcCalBasic, AdcCalScheme};
use esp_hal::peripherals::ADC1 as ADC1Peripheral;
use esp_hal::peripherals::GPIO2 as GPIO2Peripheral;
use esp_wifi::wifi::{self, WifiController};
use static_cell::StaticCell;
use heapless::Vec;
use core::mem::swap;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const ADC_READ_RATE: i32 = 200;//Hz
const PUBLISH_PERIOD: i32 = 10;//sec
const BUFFER_SIZE: usize = (ADC_READ_RATE * PUBLISH_PERIOD) as usize;

type Buffer = Vec<u16, BUFFER_SIZE>;

// Channel for passing full buffers to publisher task
static BUFFER_CHANNEL: Channel<CriticalSectionRawMutex, Buffer, 2> = Channel::new();

// Double buffer structure
struct DoubleBuffer {
    buffer_a: Buffer,
    buffer_b: Buffer,
    current_buffer: BufferSelect,
}

#[derive(Debug, Clone, Copy)]
enum BufferSelect {
    A,
    B,
}

impl DoubleBuffer {
    fn new() -> Self {
        Self {
            buffer_a: Vec::new(),
            buffer_b: Vec::new(),
            current_buffer: BufferSelect::A,
        }
    }

    // Get mutable reference to current active buffer
    fn get_current_buffer(&mut self) -> &mut Buffer {
        match self.current_buffer {
            BufferSelect::A => &mut self.buffer_a,
            BufferSelect::B => &mut self.buffer_b,
        }
    }

    // Swap buffers and return the now-inactive buffer
    fn swap_and_take(&mut self) -> Buffer {
        let old_buffer = match self.current_buffer {
            BufferSelect::A => {
                self.current_buffer = BufferSelect::B;
                core::mem::replace(&mut self.buffer_a, Vec::new())
            }
            BufferSelect::B => {
                self.current_buffer = BufferSelect::A;
                core::mem::replace(&mut self.buffer_b, Vec::new())
            }
        };
        old_buffer
    }

    // Check if current buffer is full
    fn is_current_buffer_full(&self) -> bool {
        let current_len = match self.current_buffer {
            BufferSelect::A => self.buffer_a.len(),
            BufferSelect::B => self.buffer_b.len(),
        };
        current_len >= BUFFER_SIZE
    }
}


#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);
   
    let analog_pin = peripherals.GPIO2;
    let mut adc1_config = AdcConfig::new();
    let mut pin = adc1_config.enable_pin_with_cal::<GPIO2Peripheral, AdcCalBasic<ADC1Peripheral>>(
        analog_pin,
        Attenuation::_11dB
    );
    let mut adc1 = Adc::new(peripherals.ADC1, adc1_config);

    spawner.spawn(adc_task(adc1,pin)).unwrap();
    spawner.spawn(publisher_task()).unwrap();
}


#[embassy_executor::task]
async fn adc_task(mut adc: Adc<'static, ADC1Peripheral<'static>, Blocking>, 
                  mut pin: AdcPin<GPIO2Peripheral<'static>, ADC1Peripheral<'static>, AdcCalBasic<ADC1Peripheral<'static>>>) {
   
    println!("Starting ADC task...");

    let mut double_buffer = DoubleBuffer::new();
    let v_ref: f32 = 3.1;//3.1v for 11db attenuation

    loop {
        let adc_value: u16 = nb::block!(adc.read_oneshot(&mut pin)).unwrap();
        let mv: u16 = ((adc_value as f32 / 4095.0) * v_ref * 1000.0) as u16;
        let current_buffer = double_buffer.get_current_buffer();
        let _ = current_buffer.push(mv);
        if double_buffer.is_current_buffer_full() {
            let full_buffer = double_buffer.swap_and_take();
            if BUFFER_CHANNEL.try_send(full_buffer).is_err() {
               println!("[adc task] WARNING: Buffer dropped!"); 
            }
        }
        embassy_time::Timer::after_millis(5).await;
    }
}

#[embassy_executor::task]
async fn publisher_task() {
    println!("Starting publisher task...");
    loop {
        let buffer = BUFFER_CHANNEL.receive().await;
        
        for sample in buffer {
            println!("{}", sample);
        }

    }
}       
