use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Mutex, time::Duration};
use sysinfo::System;

#[derive(Serialize, Deserialize)]
pub struct SystemStatusObject {
    memory: u64,
    swap: u64,
    cpu: f32,
}

#[derive(Serialize, Deserialize)]
struct CpuInfoObject {
    arch: String,
    num: usize,
    name: String,
    base_freq: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct OsInfo {
    name: String,
    kernel_version: String,
    os_version: String,
    host_name: String,
}

#[derive(Serialize, Deserialize)]
pub struct SystemInfoObject {
    total_ram: u64,
    total_swap: u64,
    cpu: CpuInfoObject,
    os: OsInfo,
}

static SYSTEM: Lazy<Mutex<System>> = Lazy::new(|| {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let result = Mutex::new(sys);

    result
});

static OS_INFO: Lazy<OsInfo> = Lazy::new(|| OsInfo {
    name: System::name().unwrap_or_default(),
    kernel_version: System::kernel_version().unwrap_or_default(),
    os_version: System::os_version().unwrap_or_default(),
    host_name: System::host_name().unwrap_or_default(),
});

static TOTAL_MEMORY: Lazy<u64> = Lazy::new(|| SYSTEM.lock().unwrap().total_memory());

static TOTAL_SWAP: Lazy<u64> = Lazy::new(|| SYSTEM.lock().unwrap().total_swap());

pub static USED_RESOURCE: Lazy<Mutex<VecDeque<SystemStatusObject>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(10)));

pub fn get_system_info() -> SystemInfoObject {
    let mut system = System::new();
    system.refresh_memory();
    system.refresh_cpu_all();

    let cpu_info = CpuInfoObject {
        arch: std::env::consts::ARCH.to_string(),
        num: system.cpus().len(),
        name: system.cpus()[0].brand().to_string(),
        base_freq: format!("{:.2} GHz", (system.cpus()[0].frequency() as f32 / 1000.0)),
    };

    SystemInfoObject {
        total_ram: *TOTAL_MEMORY,
        total_swap: *TOTAL_SWAP,
        cpu: cpu_info,
        os: OS_INFO.clone(),
    }
}
pub async fn sys_info_getter() {
    loop {
        {
            let mut sys = SYSTEM.lock().unwrap();
            sys.refresh_cpu_usage();
            sys.refresh_memory();

            let mut res_lock = USED_RESOURCE.lock().unwrap();

            if res_lock.len() == 10 {
                res_lock.pop_front();
            }

            let system_status = SystemStatusObject {
                memory: sys.used_memory(),
                swap: sys.used_swap(),
                cpu: sys.global_cpu_usage(),
            };

            res_lock.push_back(system_status);
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
