#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::delay::{Delay};
use esp_hal::analog::adc::{Adc, AdcPin, AdcConfig, Attenuation};
use esp_wifi::wifi::{self, WifiController};
use static_cell::StaticCell;


#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);
    
    let analog_pin = peripherals.GPIO2;
    let mut adc1_config = AdcConfig::new();
    let mut pin = adc1_config.enable_pin(
        analog_pin,
        Attenuation::_6dB  
    );
    let mut adc1 = Adc::new(peripherals.ADC1, adc1_config);
    let v_ref = 2.2;//2.2v for 6db attenuation
    
    loop {
        let adc_value: u16 = nb::block!(adc1.read_oneshot(&mut pin)).unwrap();
        let mv = (adc_value as f32 / 4095.0) * v_ref * 1000 as f32;
        println!("adc_value = {}, mv = {}", adc_value, mv);
        embassy_time::Timer::after_secs(1).await;
    }
}

