#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use blocking_network_stack::Stack;
use esp_hal::main;
use embedded_io::Write;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_wifi::wifi::{self, WifiController, WifiDevice};
use static_cell::StaticCell;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::wire::{DhcpOption, IpAddress};
use smoltcp::time::Instant as SmolInstant;
use heapless::Vec;

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
    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG);

    // wifi setup
    static WIFI_INIT_CELL: StaticCell<esp_wifi::EspWifiController> = StaticCell::new();
    let wifi_init = WIFI_INIT_CELL.init(esp_wifi::init(timg0.timer0, rng.clone()).unwrap());
    let (controller, interfaces) = esp_wifi::wifi::new(wifi_init, peripherals.WIFI).unwrap();
    let mut device = interfaces.sta;

    // Network setup
    let addr = smoltcp::wire::HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress::from_bytes(&device.mac_address()));
    let config_dev = Config::new(addr);
    let iface = Interface::new(config_dev, &mut device, SmolInstant::from_millis(0));
    
    // i have no idea
    static SOCKETS_STORAGE: StaticCell<[SocketStorage<'static>; 3]> = StaticCell::new();
    let socket_set_entries = SOCKETS_STORAGE.init([SocketStorage::EMPTY, SocketStorage::EMPTY, SocketStorage::EMPTY]).as_mut();
    let mut socket_set = SocketSet::new(&mut socket_set_entries[..]);
    let mut dhcp_socket = smoltcp::socket::dhcpv4::Socket::new();

    // we can set a hostname here (or add other DHCP options)
    dhcp_socket.set_outgoing_options(&[DhcpOption {
        kind: 12,
        data: b"implRust",
    }]);
    socket_set.add(dhcp_socket);

    let now = || esp_hal::time::Instant::now().duration_since_epoch().as_millis();
    let stack = Stack::new(
        iface,
        device,
        socket_set,
        now,
        rng.random(),
    );

    connect_wifi(controller, stack);

    loop {
        println!("Toggle");
        led.toggle();
        let delay_start = Instant::now();
        while delay_start.elapsed() < Duration::from_millis(2000) {}
    }
}

fn connect_wifi(mut controller: WifiController<'static>, stack: Stack<'static, WifiDevice<'static>>) {
    let client_config = wifi::Configuration::Client(wifi::ClientConfiguration {
        ssid: SSID.try_into().unwrap(),
        password: PASSWORD.try_into().unwrap(),
        ..Default::default()
    });

    let res = controller.set_configuration(&client_config);
    println!("Wifi config: {:?}", res);

    controller.start().unwrap();

    let c = controller.connect();
    println!("Wifi connect {:?}", c);

    println!("Waiting to connect...");
    loop {
        match controller.is_connected() {
            Ok(true) => break,
            Ok(false) => {}
            Err(err) => panic!("{:?}", err),
        }
    }
    println!("Connected!");

    println!("Wait for IP address");
    loop {
        stack.work();
        if stack.is_iface_up() {
            println!("IP acquired: {:?}", stack.get_ip_info());
            break;
        }
    }

    static RX_BUFF: StaticCell<[u8; 4000]> = StaticCell::new();
    static TX_BUFF: StaticCell<[u8; 4000]> = StaticCell::new();
    let rx_buffer: &'static mut [u8; 4000] = RX_BUFF.init([0;4000]);
    let tx_buffer: &'static mut [u8; 4000] = TX_BUFF.init([0;4000]);

    let mut socket = stack.get_socket(rx_buffer, tx_buffer);
    let remote_addr = IpAddress::v4(172, 20, 10, 4);

    let mut v: Vec<u8,4000> = Vec::new();

    for n in 0..4000 {
        let i: u8 = (n%256) as u8;
        v.push(i).expect("error adding to v");
    }

    loop {
        if !socket.is_open() {
            println!("Opening socket");
            socket.work();
            socket.open(remote_addr, 8080).expect("Failed to open socket");
        }

        match socket.write(&v) {
            Ok(_) => {
                if let Err(e) = socket.flush() {
                    println!("Flush failed: {:?}", e);
                    println!("Reconnecting...");
                    continue;
                }
                println!("Sent buffer!");
            }
            Err(e) => {
                println!("Write failed: {:?}", e);
                println!("Reconnecting...");
                continue;
            }
        }

        let delay_start = Instant::now();
        while delay_start.elapsed() < Duration::from_millis(2000) {}
    }
}

