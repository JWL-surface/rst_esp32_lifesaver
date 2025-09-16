#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use blocking_network_stack::Stack;
use embassy_executor::Spawner;
use embedded_io::{Read, Write};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_println::println;
use esp_hal::time::{Duration, Instant};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_wifi::wifi::{self, WifiController, WifiDevice};
use static_cell::StaticCell;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::wire::{DhcpOption, IpAddress};
use smoltcp::socket::{self, dhcpv4, tcp};
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
    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG);
    esp_hal_embassy::init(timer0.alarm0);

    let mut led = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());

    // wifi setup
    static WIFI_INIT_CELL: StaticCell<esp_wifi::EspWifiController> = StaticCell::new();
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    // let wifi_init = WIFI_INIT_CELL.init(esp_wifi::init(timg0.timer0, esp_hal::rng::Rng::new(peripherals.RNG)).unwrap());

    let wifi_init = WIFI_INIT_CELL.init(esp_wifi::init(timg0.timer0, rng.clone()).unwrap());

    let (mut controller, interfaces) = esp_wifi::wifi::new(wifi_init, peripherals.WIFI).unwrap();
    let mut device = interfaces.sta;

    // Network setup
    let addr = smoltcp::wire::HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress::from_bytes(&device.mac_address()));
    let mut config_dev = Config::new(addr);
    let mut iface = Interface::new(config_dev, &mut device, SmolInstant::from_millis(0));
    
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
    let mut stack = Stack::new(
        iface,
        device,
        socket_set,
        now,
        rng.random(),
    );

    spawner.spawn(connect_wifi(controller, stack)).unwrap();
    spawner.spawn(blink_led(led)).unwrap();
    // let _ = spawner.spawn(net_task(iface, device, sockets));
}

#[embassy_executor::task]
async fn connect_wifi(mut controller: WifiController<'static>, mut stack: Stack<'static, WifiDevice<'static>>) {
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

    static RX_BUFF: StaticCell<[u8; 1536]> = StaticCell::new();
    static TX_BUFF: StaticCell<[u8; 1536]> = StaticCell::new();
    let mut rx_buffer: &'static mut [u8; 1536] = RX_BUFF.init([0;1536]);
    let mut tx_buffer: &'static mut [u8; 1536] = TX_BUFF.init([0;1536]);
    let mut socket = stack.get_socket(rx_buffer, tx_buffer);

    println!("Opening socket");
    socket.work();
    
    let remote_addr = IpAddress::v4(172, 20, 10, 7);
    socket.open(remote_addr, 8080).unwrap();

    loop {
        socket
            .write(b"Hello from ESP32\r\n")
            .unwrap();
        socket.flush().unwrap();

        embassy_time::Timer::after_secs(2).await;
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
