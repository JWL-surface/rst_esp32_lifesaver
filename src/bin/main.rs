#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_wifi::wifi::{self, WifiController};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = "Iphone";
const PASSWORD: &str = "12345678";

#[main]
fn main() -> ! {
    // generator version: 0.5.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let mut led = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let init = esp_wifi::init(timg0.timer0, esp_hal::rng::Rng::new(peripherals.RNG)).unwrap();

    let (mut controller, interfaces) = esp_wifi::wifi::new(&init, peripherals.WIFI).unwrap();
    let mut device = interfaces.sta;

    // config wifi
    let client_config = wifi::Configuration::Client(wifi::ClientConfiguration {
    ssid: SSID.try_into().unwrap(),
    password: PASSWORD.try_into().unwrap(),
    ..Default::default()
    });
    let res = controller.set_configuration(&client_config);
    println!("Wifi config: {:?}", res);

    // Start the wifi controller
    controller.start().unwrap();

    let c = controller.connect();
    println!("wifi_connect {:?}", c);

    println!("Waiting to connect...");
    loop {
        match controller.is_connected() {
            Ok(true) => break,
            Ok(false) => {}
            Err(err) => panic!("{:?}", err),
        }
    }
    println!("Connected!");

    loop {
        led.toggle();
        let delay_start = Instant::now();
        while delay_start.elapsed() < Duration::from_millis(500) {}
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-rc.0/examples/src/bin
}
