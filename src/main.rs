mod tm1637;
mod wifi;

use std::{thread, time::Duration};
use std::rc::Rc;
use core::cell::RefCell;

use anyhow::Result;
use esp_idf_hal::{
    delay::FreeRtos,
    i2c::{I2cConfig, I2cDriver},
    prelude::Peripherals,
    units::FromValueType,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::EspDefaultNvsPartition,
};
use log::info;

// OLED 相关库
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Text, Alignment},
};

// 温湿度传感器库
use shtcx::{self, PowerMode};

// 引入不同版本的 embedded-hal traits
use embedded_hal::i2c::I2c as I2c1;         // ehal 1.0
use embedded_hal_02::blocking::i2c::Write as I2c0Write; // ehal 0.2

// 自定义共享 I2C 包装器，使用 Rc 共享所有权
struct SharedI2c(Rc<RefCell<I2cDriver<'static>>>); // 注意：'static 实际上是针对 Rc 内部数据的，但 I2cDriver 的实际生命周期由 Rc 管理

// 实现 ehal 1.0 的 ErrorType
impl embedded_hal::i2c::ErrorType for SharedI2c {
    type Error = <I2cDriver<'static> as embedded_hal::i2c::ErrorType>::Error;
}

// 实现 ehal 1.0 的 I2c trait（用于 shtcx）
impl I2c1 for SharedI2c {
    fn read(&mut self, address: u8, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let mut driver = self.0.borrow_mut();
        I2c1::read(&mut *driver, address, buffer)
    }

    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut driver = self.0.borrow_mut();
        I2c1::write(&mut *driver, address, bytes)
    }

    fn write_read(
        &mut self,
        address: u8,
        bytes: &[u8],
        buffer: &mut [u8],
    ) -> Result<(), Self::Error> {
        let mut driver = self.0.borrow_mut();
        I2c1::write_read(&mut *driver, address, bytes, buffer)
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        let mut driver = self.0.borrow_mut();
        I2c1::transaction(&mut *driver, address, operations)
    }
}

// 实现 ehal 0.2 的 blocking::i2c::Write（用于 ssd1306）
impl I2c0Write for SharedI2c {
    type Error = <I2cDriver<'static> as I2c0Write>::Error;

    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut driver = self.0.borrow_mut();
        I2c0Write::write(&mut *driver, addr, bytes)
    }
}

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // --- 外设初始化 ---
    let peripherals = Peripherals::take().unwrap();
    let _sysloop = EspSystemEventLoop::take()?; // 如果不使用 Wi-Fi，可以忽略
    let _nvs = EspDefaultNvsPartition::take()?; // 如果不使用 NVS，可以忽略

    // --- 初始化 I2C 总线 (GPIO8=SDA, GPIO9=SCL) ---
    let sda = peripherals.pins.gpio8;
    let scl = peripherals.pins.gpio9;
    let config = I2cConfig::new().baudrate(400u32.kHz().into());
    let i2c_driver = I2cDriver::new(peripherals.i2c0, sda, scl, &config)?;

    // 将 I2cDriver 放入 Rc<RefCell> 中，实现共享所有权
    let i2c_rc = Rc::new(RefCell::new(i2c_driver));

    // --- 温湿度传感器 SHTC3 ---
    let sht_i2c = SharedI2c(i2c_rc.clone());
    let mut sht = shtcx::shtc3(sht_i2c);
    let device_id = sht.device_identifier().unwrap();
    info!("SHTC3 device ID: {:#02x}", device_id);

    // --- OLED 显示屏 (0.96寸, SSD1306) ---
    let oled_i2c = SharedI2c(i2c_rc.clone());
    let interface = I2CDisplayInterface::new(oled_i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();

    // 初始化 OLED（注意 DisplayError 未实现 std::error::Error，需手动转换）
    display.init().map_err(|e| anyhow::anyhow!("OLED init error: {:?}", e))?;
    display.clear(BinaryColor::Off).map_err(|e| anyhow::anyhow!("OLED clear error: {:?}", e))?;
    display.flush().map_err(|e| anyhow::anyhow!("OLED flush error: {:?}", e))?;
    info!("OLED initialized");

    // 定义文本样式
    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // --- 主循环 ---
    loop {
        // 触发温湿度测量
        sht.start_measurement(PowerMode::NormalMode).unwrap();
        FreeRtos::delay_ms(100); // 等待测量完成
        let measurement = sht.get_measurement_result().unwrap();

        let temp = measurement.temperature.as_degrees_celsius();
        let hum = measurement.humidity.as_percent();

        info!("Temp: {:.1} °C, Hum: {:.1} %", temp, hum);

        // 在 OLED 上显示
        display.clear(BinaryColor::Off)
            .map_err(|e| anyhow::anyhow!("OLED clear error: {:?}", e))?;

        let temp_str = format!("Temp: {:.1}C", temp);
        Text::with_alignment(&temp_str, Point::new(64, 28), text_style, Alignment::Center)
            .draw(&mut display)
            .map_err(|e| anyhow::anyhow!("OLED draw error: {:?}", e))?;

        let hum_str = format!("Hum: {:.1}%", hum);
        Text::with_alignment(&hum_str, Point::new(64, 44), text_style, Alignment::Center)
            .draw(&mut display)
            .map_err(|e| anyhow::anyhow!("OLED draw error: {:?}", e))?;

        display.flush()
            .map_err(|e| anyhow::anyhow!("OLED flush error: {:?}", e))?;

        // 每秒更新一次
        thread::sleep(Duration::from_secs(1));
    }
}