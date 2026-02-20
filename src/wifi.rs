use anyhow::Result;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use std::net::Ipv4Addr;

/// Wi-Fi 连接配置
#[derive(Debug, Clone)]
pub struct WifiConfig {
    pub ssid: &'static str,
    pub password: &'static str,
    pub auth_method: AuthMethod,
}

impl Default for WifiConfig {
    fn default() -> Self {
        Self {
            ssid: "Wokwi-GUEST",
            password: "",
            auth_method: AuthMethod::WPA2Personal,
        }
    }
}

/// Wi-Fi 连接结果，包含已连接的 EspWifi 实例和分配的 IP 地址
pub struct WifiConnection {
    pub wifi: EspWifi<'static>,
    pub ip_info: Ipv4Addr, // 或返回完整的 ip_info 结构
}

/// 连接到指定的 Wi-Fi 网络
///
/// # 参数
/// * `config` - Wi-Fi 连接配置
/// * `modem` - 用于与 Wi-Fi 模块通信的外设接口
/// * `sysloop` - 系统事件循环
/// * `nvs` - NVS 分区，用于存储配置
///
/// # 返回
/// 成功时返回 `WifiConnection`，包含已连接的 EspWifi 实例和 IP 信息。
pub fn connect_wifi(
    config: &WifiConfig,
    modem: impl Peripheral<P = esp_idf_svc::hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> Result<WifiConnection> {
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), Some(nvs))?;
    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;

    let client_config = ClientConfiguration {
        ssid: config
            .ssid
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid SSID"))?,
        password: config
            .password
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid password"))?,
        auth_method: config.auth_method,
        ..Default::default()
    };

    wifi.set_configuration(&Configuration::Client(client_config))?;
    log::info!("Starting Wi-Fi");
    wifi.start()?;

    log::info!("Scanning for Wi-Fi networks");
    let access_points = wifi.scan()?;
    log::info!("Found {} networks", access_points.len());
    for ap in access_points {
        log::debug!("{:#?}", ap);
    }

    log::info!("Connecting to Wi-Fi");
    wifi.connect()?;
    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("IP info: {:?}", ip_info);

    Ok(WifiConnection {
        wifi: esp_wifi,
        ip_info: ip_info.ip, // 假设只返回 IP 地址，可根据需要调整
    })
}
