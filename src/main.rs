mod tm1637;
mod wifi;

use std::{thread, time::Duration};

use anyhow::Result;
use chrono::{Local, Timelike};
use esp_idf_hal::{
    delay::FreeRtos,
    gpio::{PinDriver, Pull},
    i2c::{I2cConfig, I2cDriver},
    prelude::Peripherals,
    sys::{esp_timer_get_time, tzset},
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    mqtt::client::{EspMqttClient, MqttClientConfiguration, MqttProtocolVersion, QoS},
    nvs::EspDefaultNvsPartition,
    sntp::{EspSntp, SyncStatus},
};
use log::{info, warn};
use shtcx::PowerMode;
use tm1637::TM1637;
use wifi::{connect_wifi, WifiConfig};

// 导入 DelayNs trait 以使用 delay_us
use embedded_hal::delay::DelayNs;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // --- 外设初始化 ---
    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // 数码管
    let mut tm = TM1637::new(peripherals.pins.gpio8, peripherals.pins.gpio9);
    tm.set_brightness(7);
    tm.display_digits(&[0, 0, 0, 0]);

    // Wi-Fi 配置
    let wifi_config = WifiConfig {
        ssid: "ChinaNet-V2QP樊平军",
        password: "88888888",
        auth_method: esp_idf_svc::wifi::AuthMethod::WPA2Personal,
    };

    info!("Connecting to Wi-Fi...");
    let _connection = match connect_wifi(&wifi_config, peripherals.modem, sysloop, nvs) {
        Ok(conn) => {
            info!("Wi-Fi connected! IP: {}", conn.ip_info);
            conn
        }
        Err(e) => {
            log::error!("Failed to connect Wi-Fi: {}", e);
            return Err(e);
        }
    };
    use esp_idf_hal::prelude::FromValueType;

    // 温湿度传感器 (I2C)
    let sda = peripherals.pins.gpio4;
    let scl = peripherals.pins.gpio5;
    let config = I2cConfig::new().baudrate(400u32.kHz().into());
    let i2c = I2cDriver::new(peripherals.i2c0, sda, scl, &config)?;
    let mut sht = shtcx::shtc3(i2c);
    let device_id = sht.device_identifier().unwrap();
    info!("Device ID SHTC3: {:#02x}", device_id);

    // --- 超声波传感器 (HC-SR04P) 初始化 ---
   let mut trig = PinDriver::output(peripherals.pins.gpio6)?;
    let mut echo = PinDriver::input(peripherals.pins.gpio7)?;
    echo.set_pull(Pull::Down)?;

    // NTP 时间同步
    let ntp = EspSntp::new_default().unwrap();
    println!("Synchronizing with NTP Server");
    while ntp.get_sync_status() != SyncStatus::Completed {}
    println!("Time Sync Completed");
    unsafe {
        libc::setenv(c"TZ".as_ptr(), c"CST-8".as_ptr(), 1);
        tzset();
    }

    // MQTT 配置
    let mqtt_config = MqttClientConfiguration {
        client_id: Some("mqtt_f89f5814-f4e"),
        username: Some("a5ad6337-186f-13d8-f2f"),
        protocol_version: Some(MqttProtocolVersion::V3_1_1),
        network_timeout: Duration::from_secs(10),
        keep_alive_interval: Some(Duration::from_secs(60)),
        ..Default::default()
    };
    let broker_url = "mqtt://192.168.2.94";
    use esp_idf_svc::mqtt::client::EventPayload::{Error, Received};
    let mut client =
        EspMqttClient::new_cb(
            &broker_url,
            &mqtt_config,
            |message_event| match message_event.payload() {
                Received { data, details, .. } => {
                    info!("Received from MQTT: {:?}", data);
                    info!("Received from MQTT: {:?}", details);
                }
                Error(e) => warn!("Received error from MQTT: {:?}", e),
                _ => info!("Received from MQTT: {:?}", message_event.payload()),
            },
        )?;

    // --- 主循环 ---
    let mut colon_state = false;
    let mut delay = FreeRtos;
    loop {
        info!("Sending trigger");
        trig.set_high()?;
        delay.delay_us(15);   // 使用延时对象
        trig.set_low()?;

        let timeout_us = 100_000;
        let start_wait = unsafe { esp_timer_get_time() };
        while echo.is_low() {
            let now = unsafe { esp_timer_get_time() };
            if (now - start_wait) > timeout_us {
                info!("Echo timeout (no pulse)");
                break;
            }
        }

        if echo.is_high() {
            let pulse_start = unsafe { esp_timer_get_time() };
            while echo.is_high() {
                let now = unsafe { esp_timer_get_time() };
                if (now - pulse_start) > timeout_us {
                    info!("Pulse too long");
                    break;
                }
            }
            let pulse_end = unsafe { esp_timer_get_time() };
            let pulse_width = pulse_end - pulse_start;

            if pulse_width > 0 && pulse_width < timeout_us {
                let distance_cm = (pulse_width as f32) * 0.034 / 2.0;
                info!("Distance: {:.1} cm", distance_cm);
            } else {
                info!("Invalid width: {} µs", pulse_width);
            }
        }

        FreeRtos::delay_ms(500);

        // ========== 2. 温湿度采集 & MQTT 发布 ==========
        println!("[6] Reading temperature and humidity");
        sht.start_measurement(PowerMode::NormalMode).unwrap();
        FreeRtos::delay_ms(100);
        let measurement = sht.get_measurement_result().unwrap();

        #[derive(serde::Serialize)]
        struct CurrentMeasurement {
            current_temperature: f32,
            current_humidity: f32,
        }
        let payload = CurrentMeasurement {
            current_temperature: measurement.temperature.as_degrees_celsius(),
            current_humidity: measurement.humidity.as_percent(),
        };
        let payload = serde_json::to_string(&payload).unwrap();
        let topic = "devices/telemetry".to_string();

        println!("[7] Publishing MQTT: {}", payload);
        client.publish(&topic, QoS::AtMostOnce, false, payload.as_bytes())?;

        // ========== 3. 数码管显示时间 ==========
        println!("[8] Updating display");
        let now = Local::now();
        let hour = now.hour() as u8;
        let minute = now.minute() as u8;
        let digits = [hour / 10, hour % 10, minute / 10, minute % 10];
        colon_state = !colon_state;
        tm.display_time(&digits, colon_state);

        println!("[9] Sleeping 1 second");
        std::thread::sleep(Duration::from_secs(1));
    }
}
