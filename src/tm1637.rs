use esp_idf_hal::gpio::{Output, PinDriver};
use esp_idf_hal::peripheral::Peripheral;
use std::thread;
use std::time::Duration;

pub struct TM1637<'a> {
    clk: PinDriver<'a, esp_idf_hal::gpio::Gpio8, Output>,
    dio: PinDriver<'a, esp_idf_hal::gpio::Gpio9, Output>,
}

impl<'a> TM1637<'a> {
    /// 创建新的 TM1637 实例，指定 CLK 和 DIO 引脚
    pub fn new(
        clk: impl Peripheral<P = esp_idf_hal::gpio::Gpio8> + 'a,
        dio: impl Peripheral<P = esp_idf_hal::gpio::Gpio9> + 'a,
    ) -> Self {
        let mut clk = PinDriver::output(clk).unwrap();
        let mut dio = PinDriver::output(dio).unwrap();
        clk.set_high().unwrap();
        dio.set_high().unwrap();
        Self { clk, dio }
    }

    // 发送一个字节（低位在前）
    fn write_byte(&mut self, data: u8) {
        for i in 0..8 {
            self.clk.set_low().unwrap();
            if (data >> i) & 0x01 == 1 {
                self.dio.set_high().unwrap();
            } else {
                self.dio.set_low().unwrap();
            }
            self.clk.set_high().unwrap();
        }
        // 等待应答（TM1637 会在第9个时钟周期拉低 DIO）
        self.clk.set_low().unwrap();
        self.dio.set_high().unwrap(); // 释放 DIO
        self.clk.set_high().unwrap();
        thread::sleep(Duration::from_micros(10));
        self.clk.set_low().unwrap();
    }

    // 起始信号
    fn start(&mut self) {
        self.clk.set_high().unwrap();
        self.dio.set_high().unwrap();
        thread::sleep(Duration::from_micros(2));
        self.dio.set_low().unwrap();
    }

    // 停止信号
    fn stop(&mut self) {
        self.clk.set_low().unwrap();
        self.dio.set_low().unwrap();
        thread::sleep(Duration::from_micros(2));
        self.clk.set_high().unwrap();
        thread::sleep(Duration::from_micros(2));
        self.dio.set_high().unwrap();
    }

    /// 设置亮度 (0~7)
    pub fn set_brightness(&mut self, brightness: u8) {
        let brightness = brightness.min(7);
        let cmd = 0x88 | brightness; // 亮度命令：1000 1bbb
        self.start();
        self.write_byte(cmd);
        self.stop();
    }

    /// 显示四个数字（不控制冒点）
    pub fn display_digits(&mut self, digits: &[u8; 4]) {
        self.display_time(digits, false);
    }

    /// 显示四个数字，并可选择点亮中间的冒点（通过第二个数码管的小数点实现）
    ///
    /// 参数 `colon_on` 为 `true` 时点亮冒点，`false` 时熄灭。
    /// 注意：不同模块的冒点可能由不同位置的数码管小数点控制，本实现默认使用第二个数字的小数点。
    /// 如果你的模块冒点由第三个数字控制，可以将 `colon_index` 改为 2。
    pub fn display_time(&mut self, digits: &[u8; 4], colon_on: bool) {
        // 数据命令：写数据到显示寄存器，自动地址增加
        self.start();
        self.write_byte(0x40);
        self.stop();

        // 地址命令：从 0xC0 开始（第一个 digit 的地址）
        self.start();
        self.write_byte(0xC0);

        // 发送四个数字的段码
        for (i, &d) in digits.iter().enumerate() {
            let mut seg = match d {
                0 => 0x3F,
                1 => 0x06,
                2 => 0x5B,
                3 => 0x4F,
                4 => 0x66,
                5 => 0x6D,
                6 => 0x7D,
                7 => 0x07,
                8 => 0x7F,
                9 => 0x6F,
                10 => 0x77, // A
                11 => 0x7C, // b
                12 => 0x39, // C
                13 => 0x5E, // d
                14 => 0x79, // E
                15 => 0x71, // F
                _ => 0x00,
            };
            // 如果冒点亮，且当前是第二个数字（索引1），则加上小数点
            if colon_on && i == 1 {
                seg |= 0x80; // 点亮小数点（bit7）
            }
            self.write_byte(seg);
        }
        self.stop();

        // 亮度已在初始化时设置，此处不必重复，但保留也无妨
        // self.set_brightness(7);
    }
}
