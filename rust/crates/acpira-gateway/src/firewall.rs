//! Windows Firewall and the LAN path. The first time a program listens, Windows asks whether to let it in. A prompt
//! that is dismissed or never seen (the gateway starts from a login task, often behind other windows) leaves inbound
//! block rules for the program, and block rules win over allow rules; a prompt answered with its defaults ticks private
//! networks only, while Windows files a new Wi-Fi as public. Either way a device on the same network cannot reach the
//! gateway, the code pairing (LAN only) fails, and a QR pairing has the relay alone. Outbound traffic, the relay
//! included, is never affected.
//!
//! `blocked` reads the rules that apply to the networks this computer is on now; `allow` replaces the program's inbound
//! rules with one allow rule for every network type, written by an elevated copy of this program (`acpira gateway
//! allow-lan`, behind UAC). Both go by the gateway's own path, which stays the same across upgrades (devices/stage.rs),
//! and both talk to the firewall's COM API in-process, never through a hidden PowerShell: security software (Huorong's
//! system hardening, for one) stops a background program that starts one with an encoded script. Other systems have no
//! such gate here.

/// Whether Windows Firewall turns away devices on the current network; false when it cannot tell
pub async fn blocked() -> bool {
  imp::blocked().await
}

/// Let devices on any network in: the program's inbound rules replaced by one allow rule, after a UAC prompt. Fails
/// when the prompt is declined
pub async fn allow() -> anyhow::Result<()> {
  imp::allow().await
}

/// The elevated half of `allow` (`acpira gateway allow-lan`): writes the rules, which only an administrator may
pub fn write_rules() -> anyhow::Result<()> {
  imp::write_rules()
}

#[cfg(windows)]
mod imp {
  use anyhow::{Context, Result, bail};
  use windows::Win32::Foundation::VARIANT_TRUE;
  use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NET_FW_ACTION_ALLOW, NET_FW_ACTION_BLOCK, NET_FW_IP_PROTOCOL_ANY, NET_FW_IP_PROTOCOL_UDP, NET_FW_PROFILE2_ALL,
    NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN, NetFwPolicy2, NetFwRule,
  };
  use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IDispatch,
  };
  use windows::Win32::System::Ole::IEnumVARIANT;
  use windows::Win32::System::Variant::VARIANT;
  use windows::core::{BSTR, Interface};
  use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
  use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
  use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
  use windows_sys::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
  use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

  /// What the elevated copy is started with
  const ALLOW_ARGS: &str = "gateway allow-lan";

  /// COM on the calling thread for as long as this lives; a thread that already has it keeps its own apartment.
  /// Declared before every interface it serves, so it is dropped after them
  struct Com(bool);

  impl Com {
    fn enter(apartment: COINIT) -> Com {
      // SAFETY: balanced by CoUninitialize in Drop when this call initialized COM (S_OK or S_FALSE)
      Com(unsafe { CoInitializeEx(None, apartment) }.is_ok())
    }
  }

  impl Drop for Com {
    fn drop(&mut self) {
      if self.0 {
        // SAFETY: balances the CoInitializeEx that succeeded on this thread in enter
        unsafe { CoUninitialize() };
      }
    }
  }

  fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
  }

  /// This program's path, as the rules name it
  fn program() -> Result<String> {
    Ok(std::env::current_exe().context("the gateway's own path")?.to_string_lossy().into_owned())
  }

  /// A rule's program path with its `%NAME%` references filled in (rules keep `%SystemRoot%\…` and the like as written)
  fn expand(raw: &str) -> String {
    let src = wide(raw);
    let mut buf = vec![0u16; 512];
    loop {
      // SAFETY: `src` is NUL-terminated and `buf` holds the length passed; the call writes at most that many units
      let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
      match n {
        0 => return raw.to_owned(),
        // The count includes the terminator; one above the buffer's size asks for that much room
        n if n <= buf.len() => return String::from_utf16_lossy(&buf[..n - 1]),
        n => buf.resize(n, 0),
      }
    }
  }

  /// The inbound rules naming `program`, whatever else they say. A rule that cannot be read is passed over
  fn program_rules(policy: &INetFwPolicy2, program: &str) -> Result<Vec<INetFwRule>> {
    let program = program.to_lowercase();
    // SAFETY: plain COM calls on live interfaces; `slot` is a valid, empty VARIANT for each `Next`, cleared when dropped
    unsafe {
      let walk: IEnumVARIANT = policy.Rules()?._NewEnum()?.cast()?;
      let mut found = Vec::new();
      loop {
        let mut slot = [VARIANT::default()];
        let mut got = 0u32;
        // The end is S_FALSE with nothing fetched
        if walk.Next(&mut slot, &mut got).is_err() || got == 0 {
          return Ok(found);
        }
        let Ok(rule) = IDispatch::try_from(&slot[0]).and_then(|d| d.cast::<INetFwRule>()) else { continue };
        if rule.Direction().ok() != Some(NET_FW_RULE_DIR_IN) {
          continue;
        }
        let Ok(app) = rule.ApplicationName() else { continue };
        if !app.is_empty() && expand(&app.to_string()).to_lowercase() == program {
          found.push(rule);
        }
      }
    }
  }

  // The rules naming this program, as they apply to the current network types (reading needs no elevation). A network
  // type the firewall is off for lets everything in
  fn turned_away() -> Result<bool> {
    let _com = Com::enter(COINIT_MULTITHREADED);
    // SAFETY: plain COM calls on interfaces created here, on a thread that has COM for the whole body
    unsafe {
      let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)?;
      let current = policy.CurrentProfileTypes()?;
      let mut on = false;
      for kind in [NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC] {
        if current & kind.0 != 0 && policy.get_FirewallEnabled(kind)?.as_bool() {
          on = true;
        }
      }
      Ok(on && rules_turn_away(&policy, current, &program()?)?)
    }
  }

  /// What the rules naming `program` say for the `current` network types: a block rule, or no allow rule at all, turns
  /// devices away. UDP only: iroh speaks QUIC
  fn rules_turn_away(policy: &INetFwPolicy2, current: i32, program: &str) -> Result<bool> {
    let (mut allow, mut block) = (false, false);
    for rule in program_rules(policy, program)? {
      // SAFETY: property reads on a live rule, on the caller's COM thread
      unsafe {
        if !rule.Enabled()?.as_bool() || rule.Profiles()? & current == 0 {
          continue;
        }
        let protocol = rule.Protocol()?;
        if protocol != NET_FW_IP_PROTOCOL_UDP.0 && protocol != NET_FW_IP_PROTOCOL_ANY.0 {
          continue;
        }
        if rule.Action()? == NET_FW_ACTION_BLOCK { block = true } else { allow = true }
      }
    }
    Ok(block || !allow)
  }

  pub async fn blocked() -> bool {
    matches!(tokio::task::spawn_blocking(turned_away).await, Ok(Ok(true)))
  }

  // Run elevated: every inbound rule for this program goes (the prompt's block rules among them), then one allow rule
  // for every network type
  pub fn write_rules() -> Result<()> {
    let _com = Com::enter(COINIT_MULTITHREADED);
    let program = program()?;
    // SAFETY: plain COM calls on interfaces created here, on a thread that has COM for the whole body
    unsafe {
      let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).context("Windows Firewall")?;
      let rules = policy.Rules()?;
      // `Remove` goes by name, and the prompt's rules share theirs with each other and with any other copy of the
      // program: each one leaves under a name nothing else has
      for (i, old) in program_rules(&policy, &program)?.into_iter().enumerate() {
        let name = BSTR::from(format!("Acpira (replaced {}-{i})", std::process::id()));
        old.SetName(&name).context("an existing rule")?;
        rules.Remove(&name).context("an existing rule")?;
      }
      let rule: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)?;
      rule.SetName(&BSTR::from("Acpira"))?;
      rule.SetDescription(&BSTR::from("Lets paired phones on this network reach Acpira remote access"))?;
      rule.SetApplicationName(&BSTR::from(program))?;
      rule.SetProtocol(NET_FW_IP_PROTOCOL_UDP.0)?;
      rule.SetDirection(NET_FW_RULE_DIR_IN)?;
      rule.SetProfiles(NET_FW_PROFILE2_ALL.0)?;
      rule.SetAction(NET_FW_ACTION_ALLOW)?;
      rule.SetEnabled(VARIANT_TRUE)?;
      rules.Add(&rule).context("the allow rule")?;
    }
    Ok(())
  }

  /// Starts this program elevated with `ALLOW_ARGS` and waits for it: the `runas` verb raises the UAC prompt, and the
  /// copy's exit code says whether the rules were written
  fn elevated() -> Result<()> {
    let _com = Com::enter(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    let (verb, file, args) = (wide("runas"), wide(&program()?), wide(ALLOW_ARGS));
    // SAFETY: a zeroed SHELLEXECUTEINFOW is valid; the strings outlive the call
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    // Synchronous (this thread has no message loop), the process handle kept to wait on, no error dialog
    info.fMask = SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = args.as_ptr();
    // The copy is a console program with nothing to show
    info.nShow = SW_HIDE;
    // SAFETY: `info` is initialized above and the strings it points at outlive the call
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
      // SAFETY: reads the calling thread's last error, set by the failed call above
      if unsafe { GetLastError() } == ERROR_CANCELLED {
        bail!("declined");
      }
      bail!("{}", std::io::Error::last_os_error());
    }
    if info.hProcess.is_null() {
      bail!("the elevated step did not start");
    }
    let mut code = 1u32;
    // SAFETY: `hProcess` is the live handle SEE_MASK_NOCLOSEPROCESS returned, closed once here
    let read = unsafe {
      WaitForSingleObject(info.hProcess, INFINITE);
      let read = GetExitCodeProcess(info.hProcess, &mut code);
      CloseHandle(info.hProcess);
      read
    };
    if read == 0 || code != 0 {
      bail!("the rules were not written");
    }
    Ok(())
  }

  pub async fn allow() -> Result<()> {
    tokio::task::spawn_blocking(elevated).await?
  }

  #[cfg(test)]
  mod tests {
    #[test]
    fn rule_paths_have_their_references_filled_in() {
      assert_eq!(super::expand(r"C:\plain\acpira.exe"), r"C:\plain\acpira.exe");
      let expanded = super::expand(r"%SystemRoot%\system32\svchost.exe");
      assert!(!expanded.contains('%') && expanded.ends_with(r"\system32\svchost.exe"), "{expanded}");
    }

    /// Against this machine's own firewall: the read needs no elevation and gives an answer either way
    #[test]
    fn the_rules_are_read_without_elevation() {
      let read = super::turned_away();
      assert!(read.is_ok(), "{read:?}");
    }

    /// Every rule of this machine walked: none names the test binary, and no allow rule turns devices away
    #[test]
    fn a_program_no_rule_names_is_turned_away() {
      let _com = super::Com::enter(super::COINIT_MULTITHREADED);
      // SAFETY: plain COM calls on an interface created here, on a thread that has COM for the whole body
      let answer = unsafe {
        super::CoCreateInstance::<_, super::INetFwPolicy2>(&super::NetFwPolicy2, None, super::CLSCTX_INPROC_SERVER)
          .map_err(anyhow::Error::from)
          .and_then(|policy| super::rules_turn_away(&policy, policy.CurrentProfileTypes()?, &super::program()?))
      };
      assert!(matches!(answer, Ok(true)), "{answer:?}");
    }
  }
}

#[cfg(not(windows))]
mod imp {
  pub async fn blocked() -> bool {
    false
  }

  pub async fn allow() -> anyhow::Result<()> {
    Ok(())
  }

  pub fn write_rules() -> anyhow::Result<()> {
    anyhow::bail!("Windows only")
  }
}
