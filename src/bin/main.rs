#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use bytemuck::cast_slice;
use blocking_network_stack::Stack;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, WithTimeout};
use embedded_io::Write;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig, Input, InputConfig};
use esp_println::println;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::Blocking;
use esp_wifi::wifi::{self, WifiController, WifiDevice};
use static_cell::StaticCell;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::wire::{DhcpOption, IpAddress};
use smoltcp::time::Instant as SmolInstant;
use esp_hal::analog::adc::{Adc, AdcPin, AdcConfig, Attenuation, AdcCalBasic};
use esp_hal::peripherals::ADC1 as ADC1Peripheral;
use esp_hal::peripherals::GPIO2 as GPIO2Peripheral;
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

// Shared signal for sensor state
static SENSOR_CONNECTED: Signal<CriticalSectionRawMutex, bool> = Signal::new();

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

    fn clear_current_buffer(&mut self) {
        self.get_current_buffer().clear();
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
    // generator version: 0.5.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG);
    esp_hal_embassy::init(timer0.alarm0);


    //hr sensor stuff
    let analog_pin = peripherals.GPIO2;
    let mut adc1_config = AdcConfig::new();
    let pin = adc1_config.enable_pin_with_cal::<GPIO2Peripheral, AdcCalBasic<ADC1Peripheral>>(
        analog_pin,
        Attenuation::_11dB
    );
    let adc1 = Adc::new(peripherals.ADC1, adc1_config);
    let lo_min_pin =  Input::new(peripherals.GPIO3, InputConfig::default());
    let lo_plus_pin = Input::new(peripherals.GPIO4, InputConfig::default());

    // LED
    let led = Output::new(peripherals.GPIO21, Level::High, OutputConfig::default());

    // wifi setup
    static WIFI_INIT_CELL: StaticCell<esp_wifi::EspWifiController> = StaticCell::new();
    let timg0 = TimerGroup::new(peripherals.TIMG0);

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

    spawner.spawn(connect_wifi(controller, stack)).unwrap();
    spawner.spawn(adc_task(adc1, pin, lo_min_pin, lo_plus_pin)).unwrap();
    spawner.spawn(blink_led(led)).unwrap();
}

#[embassy_executor::task]
async fn connect_wifi(mut controller: WifiController<'static>, stack: Stack<'static, WifiDevice<'static>>) {
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

    loop {
        if !socket.is_open() {
            println!("Opening socket");
            socket.work();
            socket.open(remote_addr, 8080).expect("Failed to open socket");
        }

        let future = BUFFER_CHANNEL.receive().with_timeout(Duration::from_millis(0));

        match future.await {
            Ok(buffer) => {
                let buffer_as_slice: &[u16] = buffer.as_slice();
                let sending_buff: &[u8] = cast_slice(buffer_as_slice);
                println!("Received buffer from ADC! Sending over TCP...");

                match socket.write(sending_buff) {
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
            }
            _ => {
                match socket.write(b"PING") {
                    Ok(_) => {
                        if let Err(e) = socket.flush() {
                            println!("Flush failed: {:?}", e);
                            println!("Reconnecting...");
                            continue;
                        }
                        println!("Ping!");
                    }
                    Err(e) => {
                        println!("Write failed: {:?}", e);
                        println!("Reconnecting...");
                        continue;
                    }
                }
                // break;
            }
        }
        embassy_time::Timer::after_millis(100).await;

        // let timer_fut = embassy_time::Timer::after_millis(100);

        // match select(buffer, timer_fut).await {
        //     Either::First(buffer) => {
        //         // Received buffer
        //         let buffer_as_slice: &[u16] = buffer.as_slice();
        //         let sending_buff: &[u8] = cast_slice(buffer_as_slice);
        //         println!("Received buffer from ADC! Sending over TCP...");

        //         match socket.write(sending_buff) {
        //             Ok(_) => {
        //                 if let Err(e) = socket.flush() {
        //                     println!("Flush failed: {:?}", e);
        //                     println!("Reconnecting...");
        //                     continue;
        //                 }
        //                 println!("Sent buffer!");
        //             }
        //             Err(e) => {
        //                 println!("Write failed: {:?}", e);
        //                 println!("Reconnecting...");
        //                 continue;
        //             }
        //         }

        //         break;
        //     }
        //     Either::Second(_) => {
        //         // Timer ticked, send ping to server
        //         match socket.write(b"PING") {
        //             Ok(_) => {
        //                 if let Err(e) = socket.flush() {
        //                     println!("Flush failed: {:?}", e);
        //                     println!("Reconnecting...");
        //                     continue;
        //                 }
        //             }
        //             Err(e) => {
        //                 println!("Write failed: {:?}", e);
        //                 println!("Reconnecting...");
        //                 continue;
        //             }
        //         }
        //     }
        // }
    }
}

#[embassy_executor::task]
async fn adc_task(mut adc: Adc<'static, ADC1Peripheral<'static>, Blocking>,
                  mut adc_pin: AdcPin<GPIO2Peripheral<'static>, ADC1Peripheral<'static>, AdcCalBasic<ADC1Peripheral<'static>>>,
                  lo_min_pin: Input<'static>,
                  lo_plus_pin: Input<'static>) {

    println!("Starting ADC task...");

    let mut double_buffer = DoubleBuffer::new();
    let v_ref: f32 = 3.1;//3.1v for 11db attenuation

    loop {

        if !lo_min_pin.is_high() || !lo_plus_pin.is_high() {
            SENSOR_CONNECTED.signal(true);
            let adc_value: u16 = nb::block!(adc.read_oneshot(&mut adc_pin)).unwrap();
            let mv: u16 = ((adc_value as f32 / 4095.0) * v_ref * 1000.0) as u16;
            let current_buffer = double_buffer.get_current_buffer();
            let _ = current_buffer.push(mv);
            if double_buffer.is_current_buffer_full() {
                let full_buffer = double_buffer.swap_and_take();
                if BUFFER_CHANNEL.try_send(full_buffer).is_err() {
                   println!("[adc task] WARNING: Buffer dropped!");
                }
            }
        }
        else {
            SENSOR_CONNECTED.signal(false);
            if double_buffer.get_current_buffer().len() > 0 {
                double_buffer.clear_current_buffer()
            }
        }
        embassy_time::Timer::after_millis(5).await;
    }
}


#[embassy_executor::task]
async fn blink_led(mut led: Output<'static>) {
    let mut connected = true;
    loop {
        // non-blocking check if signal changed
        if let Some(new_state) = SENSOR_CONNECTED.try_take() {
            connected = new_state;
        }

        led.toggle();
        if connected {
            embassy_time::Timer::after_millis(1000).await;
        } else {
            embassy_time::Timer::after_millis(200).await;
        }
    }
}
