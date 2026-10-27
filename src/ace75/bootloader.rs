use super::*;
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct BootloaderRejected(pub u8);

impl std::fmt::Display for BootloaderRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ACE75: загрузчик отклонил пакет {:02x}", self.0)
    }
}

impl std::error::Error for BootloaderRejected {}

#[derive(Debug, Clone)]
pub struct KeyboardFirmwarePlan {
    bytes: Vec<u8>,
    pub sha256: String,
}
impl KeyboardFirmwarePlan {
    pub fn new(bytes: Vec<u8>, expected_sha256: &str) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= u32::MAX as usize,
            "ACE75: недопустимый размер прошивки"
        );
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        ensure!(
            expected_sha256.len() == 64 && sha256.eq_ignore_ascii_case(expected_sha256),
            "ACE75: SHA-256 прошивки не совпадает с официальным образом"
        );
        Ok(Self { bytes, sha256 })
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

fn flag_packet(bytes: &[u8]) -> [u8; 64] {
    let mut result = [0; 64];
    result[..bytes.len()].copy_from_slice(bytes);
    result
}

pub fn boot_data_packet(command: u8, offset: u32, bytes: &[u8]) -> Result<[u8; 64]> {
    ensure!(
        [0x80, 0x82].contains(&command) && !bytes.is_empty() && bytes.len() <= 32,
        "ACE75: неверный пакет прошивки"
    );
    ensure!(
        offset.checked_add(bytes.len() as u32).is_some(),
        "ACE75: адрес прошивки переполнен"
    );
    let mut result = [0; 64];
    result[0] = command;
    result[1] = bytes.len() as u8;
    result[2..6].copy_from_slice(&offset.to_le_bytes());
    result[6..6 + bytes.len()].copy_from_slice(bytes);
    Ok(result)
}

fn flag_exchange(
    io: &dyn HidIo,
    packet: &[u8; 64],
    timeout_ms: i32,
    allow_no_ack: bool,
) -> Result<()> {
    io.write_output(REPORT_ID, packet)?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    for _ in 0..128 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let reply = io.read_input(remaining.as_millis().clamp(1, timeout_ms as u128) as i32)?;
        if reply.is_empty() {
            break;
        }
        if reply.first() != Some(&0) {
            continue;
        }
        ensure!(reply.len() >= 2, "ACE75: обрезанный ответ загрузчика");
        match reply[1] {
            0 => return Ok(()),
            1 => return Err(BootloaderRejected(packet[0]).into()),
            _ => continue,
        }
    }
    ensure!(
        allow_no_ack,
        "ACE75: загрузчик не подтвердил пакет {:02x}",
        packet[0]
    );
    Ok(())
}

pub struct Ace75Bootloader {
    io: Box<dyn HidIo>,
}
impl Ace75Bootloader {
    pub fn new(io: Box<dyn HidIo>) -> Self {
        Self { io }
    }

    pub fn flash_firmware(
        &mut self,
        plan: &KeyboardFirmwarePlan,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        ensure!(!plan.is_empty(), "ACE75: пустая прошивка");
        let packets = plan.bytes.len().div_ceil(32);
        let total = packets * 2 + 3;
        progress(0, total);
        // The SDK sends the currently connected Upgrader VID/PID to eraseRom.
        flag_exchange(
            self.io.as_ref(),
            &flag_packet(&[0x81, 7, 0x37, 0x38, 0x3d, 0x30, 0]),
            5000,
            false,
        )
        .context("ACE75: очистка области прошивки")?;
        progress(1, total);
        for (phase, command) in [0x80, 0x82].into_iter().enumerate() {
            for (index, chunk) in plan.bytes.chunks(32).enumerate() {
                let packet = boot_data_packet(command, (index * 32) as u32, chunk)?;
                flag_exchange(self.io.as_ref(), &packet, 1000, false).with_context(|| {
                    format!(
                        "ACE75: прошивка, команда {command:02x}, смещение {}",
                        index * 32
                    )
                })?;
                progress(1 + phase * packets + index + 1, total);
            }
        }
        flag_exchange(
            self.io.as_ref(),
            &flag_packet(&[0x83, 1, 0, 0, 0]),
            16,
            true,
        )
        .context("ACE75: завершение прошивки")?;
        progress(total - 1, total);
        std::thread::sleep(Duration::from_millis(100));
        flag_exchange(
            self.io.as_ref(),
            &flag_packet(&[0x84, 1, 0, 0, 0]),
            16,
            true,
        )
        .context("ACE75: перезапуск после прошивки")?;
        progress(total, total);
        Ok(())
    }
}
impl Ace75 {
    pub fn enter_bootloader(&mut self) -> Result<()> {
        flag_exchange(
            self.io.as_ref(),
            &flag_packet(&[0x5f, 6, 0, 0x52, 1, 0, 0, 0, 0x51, 0, 0xaa, 0xbb]),
            16,
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct BootMock {
        replies: Mutex<VecDeque<Vec<u8>>>,
        written: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl HidIo for BootMock {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            panic!("not a feature report")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            panic!("not a feature report")
        }
        fn write_output(&self, id: u8, bytes: &[u8]) -> Result<()> {
            assert_eq!(id, 0);
            self.written.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self.replies.lock().unwrap().pop_front().unwrap_or_default())
        }
    }
    #[test]
    fn firmware_plan_requires_exact_pinned_hash_before_io() {
        let bytes = vec![1, 2, 3];
        assert!(KeyboardFirmwarePlan::new(bytes.clone(), &"0".repeat(64)).is_err());
        let hash = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(KeyboardFirmwarePlan::new(bytes, &hash).unwrap().len(), 3);
        assert!(KeyboardFirmwarePlan::new(vec![], &format!("{:x}", Sha256::digest([]))).is_err());
    }
    #[test]
    fn reject_during_verification_prevents_end_and_reboot() {
        let bytes = vec![0x55; 33];
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let plan = KeyboardFirmwarePlan::new(bytes, &hash).unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut loader = Ace75Bootloader::new(Box::new(BootMock {
            replies: Mutex::new(VecDeque::from([
                vec![0, 0],
                vec![0, 0],
                vec![0, 0],
                vec![0, 1],
            ])),
            written: written.clone(),
        }));
        assert!(loader.flash_firmware(&plan, |_, _| {}).is_err());
        let writes = written.lock().unwrap();
        assert_eq!(
            writes.iter().map(|packet| packet[0]).collect::<Vec<_>>(),
            [0x81, 0x80, 0x80, 0x82]
        );
        assert_eq!(&writes[0][..7], &[0x81, 7, 0x37, 0x38, 0x3d, 0x30, 0]);
        assert_eq!(&writes[2][..7], &[0x80, 1, 32, 0, 0, 0, 0x55]);
    }
    #[test]
    fn negative_final_ack_remains_a_typed_rejection_after_transfer_verification() {
        for command in [0x83, 0x84] {
            let bytes = vec![0x55];
            let hash = format!("{:x}", Sha256::digest(&bytes));
            let plan = KeyboardFirmwarePlan::new(bytes, &hash).unwrap();
            let written = Arc::new(Mutex::new(Vec::new()));
            let mut replies = VecDeque::from(vec![vec![0, 0]; if command == 0x83 { 3 } else { 4 }]);
            replies.push_back(vec![0, 1]);
            let mut loader = Ace75Bootloader::new(Box::new(BootMock {
                replies: Mutex::new(replies),
                written: written.clone(),
            }));
            let mut transfer_verified = false;
            let error = loader
                .flash_firmware(&plan, |done, total| {
                    transfer_verified = done >= total.saturating_sub(2);
                })
                .unwrap_err();
            assert!(transfer_verified);
            // Context must retain the rejection type for the USB coordinator.
            assert_eq!(
                error.downcast_ref::<BootloaderRejected>().unwrap().0,
                command
            );
            let writes = written.lock().unwrap();
            assert_eq!(writes.last().unwrap()[0], command);
            assert_eq!(writes.len(), if command == 0x83 { 4 } else { 5 });
        }
    }
    #[test]
    fn firmware_packets_match_original_sdk_calls() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/ace75/sdk-golden-vectors.json"
        ))
        .unwrap();
        let calls = fixture["firmware"]["calls"].as_array().unwrap();
        let bytes: Vec<u8> = fixture["firmware"]["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let packets = [
            flag_packet(&[0x5f, 6, 0, 0x52, 1, 0, 0, 0, 0x51, 0, 0xaa, 0xbb]),
            flag_packet(&[0x81, 7, 0x37, 0x38, 0x3d, 0x30, 0]),
            boot_data_packet(0x80, 0, &bytes[..32]).unwrap(),
            boot_data_packet(0x80, 32, &bytes[32..]).unwrap(),
            boot_data_packet(0x82, 0, &bytes[..32]).unwrap(),
            boot_data_packet(0x82, 32, &bytes[32..]).unwrap(),
            flag_packet(&[0x83, 1, 0, 0, 0]),
            flag_packet(&[0x84, 1, 0, 0, 0]),
        ];
        assert_eq!(packets.len(), calls.len());
        for (packet, call) in packets.iter().zip(calls) {
            let expected: Vec<u8> = call["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(packet.as_slice(), expected);
        }
    }
}
