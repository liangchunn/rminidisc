use std::time::Duration;

use anyhow::{bail, Context};
use log::{debug, info};
use rusb::{Device, DeviceHandle, GlobalContext, UsbContext};

/// Sony USB vendor ID.
pub const SONY_VENDOR_ID: u16 = 0x054c;
/// Sharp USB vendor ID. Sharp devices need a different disc-title descriptor flow.
pub const SHARP_VENDOR_ID: u16 = 0x04dd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFlags {
    pub native_mono_upload: bool,
    pub native_lp_encoding: bool,
}

impl DeviceFlags {
    const fn empty() -> Self {
        Self {
            native_mono_upload: false,
            native_lp_encoding: false,
        }
    }

    const fn native_mono_upload() -> Self {
        Self {
            native_mono_upload: true,
            native_lp_encoding: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDefinition {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: &'static str,
    pub flags: DeviceFlags,
}

pub const SUPPORTED_DEVICES: &[DeviceDefinition] = &[
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x7202,
        name: "Sharp IM-MT899H",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x9013,
        name: "Sharp IM-DR400",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x9014,
        name: "Sharp IM-DR80",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0034,
        name: "Sony PCLK-XX",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0036,
        name: "Sony",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0075,
        name: "Sony MZ-N1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x007c,
        name: "Sony",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0080,
        name: "Sony LAM-1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0081,
        name: "Sony MDS-JB980/MDS-NT1/MDS-JE780",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0084,
        name: "Sony MZ-N505",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0085,
        name: "Sony MZ-S1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0086,
        name: "Sony MZ-N707",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x008e,
        name: "Sony CMT-C7NT",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0097,
        name: "Sony PCGA-MDN1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00ad,
        name: "Sony CMT-L7HD",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c6,
        name: "Sony MZ-N10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c7,
        name: "Sony MZ-N910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c8,
        name: "Sony MZ-N710/NF810",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c9,
        name: "Sony MZ-N510/N610",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00ca,
        name: "Sony MZ-NE410/NF520D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00e7,
        name: "Sony CMT-M333NT/M373NT",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00eb,
        name: "Sony MZ-NE810/NE910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0101,
        name: "Sony LAM",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0113,
        name: "Aiwa AM-NX1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x013f,
        name: "Sony MDS-S500",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x014c,
        name: "Aiwa AM-NX9",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x017e,
        name: "Sony MZ-NH1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0180,
        name: "Sony MZ-NH3D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0182,
        name: "Sony MZ-NH900",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0184,
        name: "Sony MZ-NH700/NH800",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0186,
        name: "Sony MZ-NH600",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0187,
        name: "Sony MZ-NH600D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0188,
        name: "Sony MZ-N920",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x018a,
        name: "Sony LAM-3",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x01e9,
        name: "Sony MZ-DH10P",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0219,
        name: "Sony MZ-RH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x021b,
        name: "Sony MZ-RH710/MZ-RH910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x021d,
        name: "Sony CMT-AH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x022c,
        name: "Sony CMT-AH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x023c,
        name: "Sony DS-HMD1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0286,
        name: "Sony MZ-RH1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x011a,
        name: "Sony CMT-SE7",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0148,
        name: "Sony MDS-A1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x0b28,
        product_id: 0x1004,
        name: "Kenwood MDX-J9",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04da,
        product_id: 0x23b3,
        name: "Panasonic SJ-MR250",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x04da,
        product_id: 0x23b6,
        name: "Panasonic SJ-MR270",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x0411,
        product_id: 0x0083,
        name: "Buffalo MD-HUSB",
        flags: DeviceFlags::empty(),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceSelector {
    pub vendor_id: u16,
    pub product_id: u16,
}

impl DeviceSelector {
    pub const fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id,
            product_id,
        }
    }
}

pub fn supported_device(vendor_id: u16, product_id: u16) -> Option<&'static DeviceDefinition> {
    SUPPORTED_DEVICES
        .iter()
        .find(|device| device.vendor_id == vendor_id && device.product_id == product_id)
}

/// Opens one connected supported NetMD device and claims its interface.
///
/// If exactly one supported device is connected, it is selected automatically.
/// If multiple supported devices are connected, callers must pass a selector.
pub fn open_device() -> anyhow::Result<DeviceHandle<GlobalContext>> {
    open_device_matching(None)
}

pub fn open_device_matching(
    selector: Option<DeviceSelector>,
) -> anyhow::Result<DeviceHandle<GlobalContext>> {
    let devices = rusb::devices()?
        .iter()
        .filter_map(connected_supported_device)
        .collect::<Vec<_>>();

    let devices = devices
        .into_iter()
        .filter(|connected| {
            selector
                .map(|selector| {
                    connected.definition.vendor_id == selector.vendor_id
                        && connected.definition.product_id == selector.product_id
                })
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let device = match devices.as_slice() {
        [] => {
            if let Some(selector) = selector {
                bail!(
                    "cannot find supported device {:04x}:{:04x}",
                    selector.vendor_id,
                    selector.product_id
                );
            }
            bail!("cannot find supported NetMD device");
        }
        [device] => device,
        _ => bail!(
            "multiple supported NetMD devices connected; specify one with --device <vid:pid>:\n{}",
            devices
                .iter()
                .map(ConnectedDevice::display)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };

    let device_desc = device.device.device_descriptor()?;
    let device_name = device.definition.name;
    let device_id = format!(
        "{:04x}:{:04x} {device_name}",
        device_desc.vendor_id(),
        device_desc.product_id()
    );
    let handle = device
        .device
        .open()
        .with_context(|| format!("failed to open USB device {device_id}"))?;

    if let Ok(langs) = handle.read_languages(Duration::from_secs(5)) {
        let manufacturer = langs
            .iter()
            .filter_map(|lang| {
                handle
                    .read_manufacturer_string(*lang, &device_desc, Duration::from_secs(5))
                    .ok()
            })
            .collect::<Vec<_>>();
        debug!(
            "opened {:04x}:{:04x} {} ({})",
            device_desc.vendor_id(),
            device_desc.product_id(),
            device_name,
            manufacturer.join(", ")
        );
    }

    handle
        .claim_interface(0)
        .with_context(|| format!("failed to claim USB interface 0 on {device_id}"))?;
    Ok(handle)
}

struct ConnectedDevice {
    device: Device<GlobalContext>,
    definition: &'static DeviceDefinition,
}

impl ConnectedDevice {
    fn display(&self) -> String {
        format!(
            "  {:04x}:{:04x} {}",
            self.definition.vendor_id, self.definition.product_id, self.definition.name
        )
    }
}

pub fn list_connected_devices() -> anyhow::Result<Vec<&'static DeviceDefinition>> {
    Ok(rusb::devices()?
        .iter()
        .filter_map(connected_supported_device)
        .map(|cd| cd.definition)
        .collect())
}

fn connected_supported_device(device: Device<GlobalContext>) -> Option<ConnectedDevice> {
    let desc = device.device_descriptor().ok()?;
    supported_device(desc.vendor_id(), desc.product_id())
        .map(|definition| ConnectedDevice { device, definition })
}

/// Releases the claimed interface. Mirrors the runner's previous teardown.
pub fn close_device<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    info!("closing device");
    handle.release_interface(0)?;
    Ok(())
}

/// Returns the `(vendor_id, product_id)` of the device behind a handle. Used for
/// EKB selection during secure download.
pub fn device_ids<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<(u16, u16)> {
    let desc = handle.device().device_descriptor()?;
    Ok((desc.vendor_id(), desc.product_id()))
}
