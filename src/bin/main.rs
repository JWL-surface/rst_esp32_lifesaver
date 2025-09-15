#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::str::SplitWhitespace;

use embassy_executor::Spawner;
use embedded_io::{Read, Write};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_wifi::wifi::{self, WifiController};
use heapless::vec;
use static_cell::StaticCell;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::wire::{DhcpOption, EthernetAddress, IpCidr, Ipv4Address, TcpPacket};
use smoltcp::socket::tcp;
use smoltcp::time::Instant as SmolInstant;

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

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.5.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    let mut led = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());

    // wifi setup
    static WIFI_INIT_CELL: StaticCell<esp_wifi::EspWifiController> = StaticCell::new();
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let wifi_init = WIFI_INIT_CELL.init(esp_wifi::init(timg0.timer0, esp_hal::rng::Rng::new(peripherals.RNG)).unwrap());

    let (mut controller, interfaces) = esp_wifi::wifi::new(wifi_init, peripherals.WIFI).unwrap();
    let mut device = interfaces.sta;

    // Network setup
    let m = smoltcp::wire::HardwareAddress::Ethernet(EthernetAddress([0x02,0x00,0x00,0x12,0x34,0x56]));
    let mut config = Config::new(m);
    let mut iface = Interface::new(config, &mut device, SmolInstant::from_millis(0));

    // i have no idea
    static mut SOCKETS_STORAGE: StaticCell<[SocketStorage<'static>; 4]> = StaticCell::new();
    unsafe {
        let mut sockets_store = SOCKETS_STORAGE.init([SocketStorage::EMPTY, SocketStorage::EMPTY, SocketStorage::EMPTY, SocketStorage::EMPTY]);
        let mut sockets = SocketSet::new(&mut sockets_store[..]);
    }
    
    spawner.spawn(token)

    // let mut dhcp_socket = smoltcp::socket::dhcpv4::Socket::new();

    // dhcp_socket.set_outgoing_options(&[DhcpOption {
    //     kind: 12,
    //     data: b"implRust",
    // }]);
    // socket_set.add(dhcp_socket);

    spawner.spawn(connect_wifi(controller)).unwrap();
    spawner.spawn(blink_led(led)).unwrap();

    loop {
        let now = SmolInstant::from_millis(embassy_time::Instant::now().as_millis() as i64);
        match iface.poll(now, &mut device, &mut sockets) {
            smoltcp::iface::PollResult::SocketStateChanged => {println!("SocketStateChanged");}
            smoltcp::iface::PollResult::None => { println!("None"); }
        }
        embassy_time::Timer::after_millis(100).await;
    }
}

#[embassy_executor::task]
async fn connect_wifi(mut controller: WifiController<'static>) {
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

    loop {
        embassy_time::Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn blink_led(mut led: Output<'static>) {
    loop {
        println!("Toggle");
        led.toggle();
        embassy_time::Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut iface: Interface, mut sockets: SocketSet<'static>) {
    let mut rx_buff = [0u8; 1024];
    let tcp_rx_buff = tcp::SocketBuffer::new(&mut rx_buff[..]);
    let mut tx_buff = [0u8; 1024];
    let tcp_tx_buff = tcp::SocketBuffer::new(&mut tx_buff[..]);

    let tcp_socket = tcp::Socket::new(tcp_rx_buff, tcp_tx_buff);
    let handle = sockets.add(tcp_socket);
}
