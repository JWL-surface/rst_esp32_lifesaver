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
use embedded_io::{Read, Write};
use esp_hal::gpio::{Level, Output, OutputConfig, Input, InputConfig};
use esp_println::println;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::Blocking;
use esp_hal::clock::CpuClock;
use esp_wifi::wifi::{self, WifiController, WifiDevice};
use static_cell::StaticCell;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::wire::{DhcpOption, IpAddress};
use smoltcp::time::Instant as SmolInstant;
use esp_hal::analog::adc::{Adc, AdcPin, AdcConfig, Attenuation, AdcCalBasic};
use esp_hal::peripherals::ADC1 as ADC1Peripheral;
use esp_hal::peripherals::GPIO2 as GPIO2Peripheral;
use heapless::Vec;

// Minimal base64 implementation for no_std (can be used once crypto is added)
mod base64_simple {
    use alloc::string::String;
    use alloc::vec::Vec;

    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(input: &[u8]) -> String {
        let mut result = String::new();
        let mut i = 0;

        while i < input.len() {
            let b1 = input[i];
            let b2 = if i + 1 < input.len() { input[i + 1] } else { 0 };
            let b3 = if i + 2 < input.len() { input[i + 2] } else { 0 };

            let n = ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);

            result.push(CHARSET[((n >> 18) & 63) as usize] as char);
            result.push(CHARSET[((n >> 12) & 63) as usize] as char);

            if i + 1 < input.len() {
                result.push(CHARSET[((n >> 6) & 63) as usize] as char);
            } else {
                result.push('=');
            }

            if i + 2 < input.len() {
                result.push(CHARSET[(n & 63) as usize] as char);
            } else {
                result.push('=');
            }

            i += 3;
        }

        result
    }

    pub fn decode(input: &str) -> heapless::Vec<u8, 256> {
        let mut result: heapless::Vec<u8, 256> = heapless::Vec::new();
        let input_bytes = input.as_bytes();

        let mut i = 0;
        while i < input_bytes.len() {
            let c1 = input_bytes[i];
            let c2 = if i + 1 < input_bytes.len() { input_bytes[i + 1] } else { b'=' };
            let c3 = if i + 2 < input_bytes.len() { input_bytes[i + 2] } else { b'=' };
            let c4 = if i + 3 < input_bytes.len() { input_bytes[i + 3] } else { b'=' };

            let b1 = char_to_value(c1);
            let b2 = char_to_value(c2);
            let b3 = char_to_value(c3);
            let b4 = char_to_value(c4);

            if b1 == 255 || b2 == 255 { break; }

            let n = ((b1 as u32) << 18) | ((b2 as u32) << 12) |
                   ((if c3 == b'=' { 0 } else { b3 as u32 }) << 6) |
                   (if c4 == b'=' { 0 } else { b4 as u32 });

            let _ = result.push((n >> 16) as u8);
            if c3 != b'=' {
                let _ = result.push((n >> 8) as u8);
            }
            if c4 != b'=' {
                let _ = result.push(n as u8);
            }

            i += 4;
        }

        result
    }

    fn char_to_value(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 255,
        }
    }
}

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

// Azure IoT Hub configuration
const IOT_HUB_NAME: &str = "ih-iothub01.azure-devices.net";
const DEVICE_ID: &str = "device01-piemulator";
const DEVICE_KEY: &str = "placeholder_key_base64_encoded";

// Generate Azure IoT Hub SAS token
// NOTE: For production, this needs proper HMAC-SHA256 implementation
// Using placeholder for testing MQTT protocol
fn generate_sas_token(iot_hub_name: &str, device_id: &str, _device_key: &str, expiry_secs: u64) -> alloc::string::String {
    use alloc::format;

    let now = esp_hal::time::Instant::now().duration_since_epoch().as_secs();
    let ttl = now + expiry_secs;

    // Create URI: "{iot_hub_name}/devices/{device_id}"
    let uri = format!("{}/devices/{}", iot_hub_name, device_id);

    // URL encode the URI (simple implementation - encode slashes)
    let encoded_uri = uri.replace("/", "%2F");

    // NOTE: This is a PLACEHOLDER token for testing only!
    // In production, compute proper HMAC-SHA256 signature with device key
    let placeholder_sig = "PLACEHOLDER_SIGNATURE_TESTING_ONLY";

    // Create the SAS token
    let sas_token = format!(
        "SharedAccessSignature sr={}&sig={}&se={}",
        encoded_uri, placeholder_sig, ttl
    );

    sas_token
}

// Shared signal for sensor state
static SENSOR_CONNECTED: Signal<CriticalSectionRawMutex, bool> = Signal::new();

// MQTT Protocol Implementation
mod mqtt_protocol {
    // MQTT Control Packet types
    const MQTT_CONNECT: u8 = 1;
    const MQTT_PUBLISH: u8 = 3;
    const MQTT_PINGREQ: u8 = 12;
    const MQTT_DISCONNECT: u8 = 14;

    // Encode variable length integer (up to 4 bytes)
    fn encode_remaining_length(length: u32) -> heapless::Vec<u8, 4> {
        let mut encoded: heapless::Vec<u8, 4> = heapless::Vec::new();
        let mut len = length;

        loop {
            let mut encoded_byte = (len % 128) as u8;
            len /= 128;
            if len > 0 {
                encoded_byte |= 0x80;
            }
            let _ = encoded.push(encoded_byte);
            if len == 0 {
                break;
            }
        }
        encoded
    }

    // Encode string (2 byte length + data)
    fn encode_string(s: &str) -> heapless::Vec<u8, 512> {
        let mut vec: heapless::Vec<u8, 512> = heapless::Vec::new();
        let len = s.len() as u16;
        let _ = vec.push((len >> 8) as u8);
        let _ = vec.push(len as u8);
        for byte in s.bytes() {
            let _ = vec.push(byte);
        }
        vec
    }

    // Build MQTT CONNECT packet
    pub fn build_connect(
        client_id: &str,
        username: &str,
        password: &str,
    ) -> heapless::Vec<u8, 2048> {
        let mut packet: heapless::Vec<u8, 2048> = heapless::Vec::new();

        // Variable header
        // Protocol name: "MQTT"
        let protocol_name = b"\x00\x04MQTT";
        // Protocol level: 4
        let protocol_level = 4u8;
        // Connect flags: User Name Flag | Password Flag | Clean Session
        let connect_flags = 0xC2u8; // 1100 0010
        // Keep Alive: 30 seconds
        let keep_alive_bytes = [0u8, 30u8];

        // Payload
        // Client identifier
        let client_id_encoded = encode_string(client_id);
        // Username
        let username_encoded = encode_string(username);
        // Password
        let password_encoded = encode_string(password);

        // Calculate remaining length
        let remaining = protocol_name.len() + 1 + 1 + 2 +
                       client_id_encoded.len() +
                       username_encoded.len() +
                       password_encoded.len();

        let remaining_encoded = encode_remaining_length(remaining as u32);

        // Build packet
        // Fixed header byte 1: CONNECT (type 1, flags 0000)
        let _ = packet.push(MQTT_CONNECT << 4);

        // Remaining length
        for byte in remaining_encoded.iter() {
            let _ = packet.push(*byte);
        }

        // Variable header + payload
        for &byte in protocol_name {
            let _ = packet.push(byte);
        }
        let _ = packet.push(protocol_level);
        let _ = packet.push(connect_flags);
        for byte in keep_alive_bytes.iter() {
            let _ = packet.push(*byte);
        }
        for byte in client_id_encoded.iter() {
            let _ = packet.push(*byte);
        }
        for byte in username_encoded.iter() {
            let _ = packet.push(*byte);
        }
        for byte in password_encoded.iter() {
            let _ = packet.push(*byte);
        }

        packet
    }

    // Build MQTT PUBLISH packet
    pub fn build_publish(
        topic: &str,
        payload: &[u8],
        packet_id: u16,
    ) -> heapless::Vec<u8, 8192> {
        let mut packet: heapless::Vec<u8, 8192> = heapless::Vec::new();

        // Topic name
        let topic_encoded = encode_string(topic);

        // Packet identifier (QoS 1)
        let packet_id_bytes = [(packet_id >> 8) as u8, packet_id as u8];

        // Calculate remaining length (topic + packet_id + payload)
        let remaining = topic_encoded.len() + 2 + payload.len();
        let remaining_encoded = encode_remaining_length(remaining as u32);

        // Fixed header byte 1: PUBLISH (type 3, flags 0010 for QoS 1)
        let _ = packet.push((MQTT_PUBLISH << 4) | 0x02);

        // Remaining length
        for byte in remaining_encoded.iter() {
            let _ = packet.push(*byte);
        }

        // Variable header + payload
        for byte in topic_encoded.iter() {
            let _ = packet.push(*byte);
        }
        for byte in packet_id_bytes.iter() {
            let _ = packet.push(*byte);
        }
        for byte in payload.iter() {
            let _ = packet.push(*byte);
        }

        packet
    }

    // Build MQTT DISCONNECT packet
    pub fn build_disconnect() -> heapless::Vec<u8, 2> {
        let mut packet: heapless::Vec<u8, 2> = heapless::Vec::new();
        let _ = packet.push(MQTT_DISCONNECT << 4);
        let _ = packet.push(0); // No remaining length
        packet
    }

    // Build MQTT PINGREQ packet
    pub fn build_pingreq() -> heapless::Vec<u8, 2> {
        let mut packet: heapless::Vec<u8, 2> = heapless::Vec::new();
        let _ = packet.push(MQTT_PINGREQ << 4);
        let _ = packet.push(0); // No remaining length
        packet
    }

    // Parse MQTT CONNACK response (check if connection successful)
    pub fn parse_connack(data: &[u8]) -> bool {
        // CONNACK format: [type|flags, remaining_length, reserved, return_code]
        // We need return_code == 0 to be successful
        if data.len() >= 4 {
            data[3] == 0
        } else {
            false
        }
    }

    // Parse MQTT PUBLISH response to get packet type
    pub fn get_packet_type(data: &[u8]) -> u8 {
        if data.is_empty() {
            0
        } else {
            (data[0] >> 4) & 0x0F
        }
    }
}

const ADC_READ_RATE: u32 = 125;//Hz
const ADC_READ_DELAY: u64 = 1000 / ADC_READ_RATE as u64;//ms
const PUBLISH_PERIOD: u32 = 10;//sec
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

    static RX_BUFF: StaticCell<[u8; 8000]> = StaticCell::new();
    static TX_BUFF: StaticCell<[u8; 8000]> = StaticCell::new();
    let rx_buffer: &'static mut [u8; 8000] = RX_BUFF.init([0;8000]);
    let tx_buffer: &'static mut [u8; 8000] = TX_BUFF.init([0;8000]);

    let mut socket = stack.get_socket(rx_buffer, tx_buffer);
    let remote_addr = IpAddress::v4(172, 20, 10, 4); // Local MQTT broker

    // State for MQTT connection
    let mut mqtt_connected = false;
    let mut packet_id = 1u16;

    loop {
        if !socket.is_open() {
            println!("Opening socket to MQTT broker...");
            socket.work();
            match socket.open(remote_addr, 1883) {
                Ok(_) => {
                    println!("Socket opened, sending MQTT CONNECT...");
                    // Send MQTT CONNECT packet
                    let username = alloc::format!("{}/{}/?api-version=2021-04-12", IOT_HUB_NAME, DEVICE_ID);
                    let sas_token = generate_sas_token(IOT_HUB_NAME, DEVICE_ID, DEVICE_KEY, 3600);
                    let connect_packet = mqtt_protocol::build_connect(DEVICE_ID, &username, &sas_token);

                    match socket.write(connect_packet.as_slice()) {
                        Ok(_) => {
                            match socket.flush() {
                                Ok(_) => {
                                    println!("CONNECT packet sent");
                                    // Wait briefly for CONNACK to be processed by broker
                                    embassy_time::Timer::after_millis(500).await;
                                    // Assume connection succeeded - broker will close if auth fails
                                    mqtt_connected = true;
                                }
                                Err(e) => {
                                    println!("Flush failed: {:?}", e);
                                    socket.close();
                                }
                            }
                        }
                        Err(e) => {
                            println!("Write CONNECT failed: {:?}", e);
                            socket.close();
                        }
                    }
                }
                Err(e) => {
                    println!("Failed to open socket: {:?}", e);
                }
            }
        }

        if mqtt_connected {
            let future = BUFFER_CHANNEL.receive().with_timeout(Duration::from_millis(10));

            match future.await {
                Ok(buffer) => {
                    // Create JSON payload
                    let ecg_json = {
                        use alloc::format;
                        let mut json = alloc::string::String::from("{\"ecg\":[");
                        for (i, &value) in buffer.iter().enumerate() {
                            if i > 0 {
                                json.push(',');
                            }
                            json.push_str(&alloc::format!("{}", value));
                        }
                        json.push_str("]}");
                        json
                    };

                    let topic = alloc::format!("devices/{}/messages/events/", DEVICE_ID);
                    let publish_packet = mqtt_protocol::build_publish(
                        &topic,
                        ecg_json.as_bytes(),
                        packet_id,
                    );

                    println!("Sending ECG data via MQTT (packet_id: {}), packet size: {}...", packet_id, publish_packet.len());
                    socket.work();

                    // Handle partial writes for large packets
                    let mut total_written = 0;
                    let mut offset = 0;
                    while offset < publish_packet.len() {
                        match socket.write(&publish_packet[offset..]) {
                            Ok(n) => {
                                if n > 0 {
                                    println!("Wrote {} bytes to socket", n);
                                    total_written += n;
                                    offset += n;
                                    socket.work();
                                } else {
                                    println!("Socket write returned 0, buffer full. Flushing and retrying...");
                                    socket.flush().ok();
                                    socket.work();
                                    embassy_time::Timer::after_millis(10).await;
                                    socket.work();
                                }
                            }
                            Err(e) => {
                                println!("Write failed: {:?}", e);
                                break;
                            }
                        }
                    }

                    if total_written == publish_packet.len() {
                        println!("All {} bytes written, flushing...", total_written);
                        match socket.flush() {
                            Ok(_) => {
                                socket.work();
                                println!("Published {} bytes", ecg_json.len());
                                packet_id = packet_id.wrapping_add(1);
                            }
                            Err(e) => {
                                println!("Flush failed: {:?}", e);
                            }
                        }
                    } else {
                        println!("Failed to write all bytes! Wrote {}/{}", total_written, publish_packet.len());
                    }
                }
                _ => {
                    // Send PING to keep connection alive
                    if mqtt_connected {
                        let ping_packet = mqtt_protocol::build_pingreq();
                        socket.work();
                        match socket.write(ping_packet.as_slice()) {
                            Ok(_) => {
                                socket.work();
                                match socket.flush() {
                                    Ok(_) => {
                                        socket.work();
                                        // PING sent successfully
                                    }
                                    Err(e) => {
                                        println!("PING flush failed: {:?}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                println!("PING write failed: {:?}", e);
                            }
                        }
                    }
                }
            }
        }

        socket.work();
        embassy_time::Timer::after_millis(100).await;
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

        // if !lo_min_pin.is_high() || !lo_plus_pin.is_high() 
        {
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
        // else {
        //     println!("Probes disconnected!");
        //     SENSOR_CONNECTED.signal(false);
        //     if double_buffer.get_current_buffer().len() > 0 {
        //         double_buffer.clear_current_buffer()
        //     }
        // }
        embassy_time::Timer::after_millis(ADC_READ_DELAY).await;
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
