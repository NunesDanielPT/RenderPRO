//! Pormenores do Windows que tornam os renders mais seguros:
//! - todos os ffmpeg ficam num Job Object → se o RenderPRO fechar ou crashar, morrem com ele
//!   (nunca ficam processos perdidos a escrever ficheiros);
//! - o Windows não entra em suspensão a meio de um render;
//! - prioridade baixa opcional para não roubar CPU a um player aberto na mesma máquina.

use std::sync::atomic::{AtomicUsize, Ordering};

pub static ACTIVE_RENDERS: AtomicUsize = AtomicUsize::new(0);

/// Guarda que conta renders ativos (impede a suspensão do PC enquanto existir).
pub struct ActiveGuard;
impl ActiveGuard {
    pub fn new() -> Self {
        ACTIVE_RENDERS.fetch_add(1, Ordering::SeqCst);
        keep_awake_now();
        ActiveGuard
    }
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE_RENDERS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn active() -> usize {
    ACTIVE_RENDERS.load(Ordering::SeqCst)
}

#[cfg(windows)]
mod win {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_SYSTEM_REQUIRED};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    struct Job(usize);
    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(h);
                return None;
            }
            Some(Job(h as usize))
        })
        .as_ref()
        .map(|j| j.0 as HANDLE)
    }

    pub fn adopt(pid: u32) {
        let Some(j) = job() else { return };
        unsafe {
            let p = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !p.is_null() {
                AssignProcessToJobObject(j, p);
                CloseHandle(p);
            }
        }
    }

    pub fn keep_awake() {
        unsafe {
            SetThreadExecutionState(ES_SYSTEM_REQUIRED);
        }
    }
}

/// Liga o processo filho ao Job Object do RenderPRO.
pub fn adopt_child(pid: Option<u32>) {
    #[cfg(windows)]
    if let Some(pid) = pid {
        win::adopt(pid);
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn keep_awake_now() {
    #[cfg(windows)]
    win::keep_awake();
}

/// Thread que, enquanto houver renders, diz ao Windows que o sistema está ocupado.
pub fn start_keep_awake_thread() {
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(30));
        if active() > 0 {
            keep_awake_now();
        }
    });
}

#[allow(dead_code)]
/// Flags de criação: sem janela de consola e, opcionalmente, prioridade abaixo do normal.
pub fn creation_flags(low_priority: bool) -> u32 {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    if low_priority {
        CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
    } else {
        CREATE_NO_WINDOW
    }
}

/// Espaço livre no disco onde fica `dir` (sobe até encontrar uma pasta que exista).
pub fn free_space(dir: &std::path::Path) -> Option<u64> {
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.exists() {
            return fs4::available_space(p).ok();
        }
        d = p.parent();
    }
    None
}
