use super::*;

impl Ace75 {
    pub fn read_advanced(&mut self, profile: u8) -> Result<AdvancedState> {
        validate_profile(profile)?;
        let dks = self.read_range(0xa2, profile as usize * DKS_REGION_SIZE, DKS_REGION_SIZE)?;
        let mt = self.read_range(0xa4, profile as usize * MT_REGION_SIZE, MT_REGION_SIZE)?;
        let tgl = self.read_range(0xa6, profile as usize * TGL_REGION_SIZE, TGL_REGION_SIZE)?;
        AdvancedState::decode(&dks, &mt, &tgl)
    }

    pub fn apply_advanced(&mut self, profile: u8, value: &AdvancedState) -> Result<AdvancedState> {
        validate_profile(profile)?;
        value.encode()?;
        let current = self.read_advanced(profile)?;
        let mut wanted = value.clone();
        wanted.mt_reserved = current.mt_reserved.clone();
        wanted.tgl_reserved = current.tgl_reserved.clone();
        for (wanted, current) in wanted.dks.iter_mut().zip(&current.dks) {
            wanted.reserved_masks = current.reserved_masks;
        }
        let expected = wanted.encode()?;
        let before = current.encode()?;
        // MT/TGL have 32 records followed by padding, which is left untouched.
        if expected.dks != before.dks {
            self.write_range(0xa3, profile as usize * DKS_REGION_SIZE, &expected.dks, 24)?;
        }
        if expected.mt[..192] != before.mt[..192] {
            self.write_range(
                0xa5,
                profile as usize * MT_REGION_SIZE,
                &expected.mt[..192],
                6,
            )?;
        }
        if expected.tgl[..96] != before.tgl[..96] {
            self.write_range(
                0xa7,
                profile as usize * TGL_REGION_SIZE,
                &expected.tgl[..96],
                3,
            )?;
        }
        let actual = self.read_advanced(profile)?;
        ensure!(
            actual.encode()? == expected,
            "ACE75: проверка advanced keys не прошла"
        );
        Ok(actual)
    }

    pub fn read_custom_params(&mut self, profile: u8) -> Result<CustomParams> {
        CustomParams::decode(&self.read_custom_data(
            profile,
            CUSTOM_AREA_SIZE,
            CUSTOM_PARAMS_OFFSET,
            CUSTOM_PARAMS_SIZE,
        )?)
    }

    pub fn apply_custom_params(
        &mut self,
        profile: u8,
        value: &CustomParams,
    ) -> Result<CustomParams> {
        validate_profile(profile)?;
        value.encode()?;
        let mut wanted = value.clone();
        wanted.raw = self.read_custom_params(profile)?.raw;
        let expected = wanted.encode()?;
        CustomParams::decode(&self.write_custom_data_verified(
            profile,
            CUSTOM_AREA_SIZE,
            CUSTOM_PARAMS_OFFSET,
            &expected,
        )?)
    }

    pub fn read_profile_names(&mut self) -> Result<ProfileNames> {
        ProfileNames::decode(&self.read_custom_data(
            0,
            CUSTOM_AREA_SIZE,
            PROFILE_NAMES_OFFSET,
            PROFILE_NAMES_SIZE,
        )?)
    }

    pub fn apply_profile_names(&mut self, value: &ProfileNames) -> Result<ProfileNames> {
        value.encode()?;
        let base = KeyboardBase::decode(&self.read_base()?)?;
        let timestamp = update_timestamp()?;
        let mut plans = Vec::new();
        for profile in 0..base.max_profiles {
            let before = self.read_custom_data(
                profile,
                CUSTOM_AREA_SIZE,
                PROFILE_NAMES_OFFSET,
                PROFILE_NAMES_SIZE,
            )?;
            let mut wanted = value.clone();
            wanted.raw = before.clone();
            let expected = wanted.encode()?;
            let mut metadata = self.read_feature_metadata(profile)?;
            if before != expected {
                metadata.profile_name_updated_at = timestamp;
            }
            plans.push((profile, before, expected, metadata.encode()?));
        }
        for (profile, before, expected, metadata) in &plans {
            if before == expected {
                continue;
            }
            self.write_custom_data_verified(
                *profile,
                CUSTOM_AREA_SIZE,
                PROFILE_NAMES_OFFSET,
                expected,
            )?;
            self.write_custom_data_verified(
                *profile,
                CUSTOM_AREA_SIZE,
                FEATURE_METADATA_OFFSET,
                metadata,
            )?;
        }
        self.read_profile_names()
    }

    pub fn read_feature_metadata(&mut self, profile: u8) -> Result<FeatureMetadata> {
        FeatureMetadata::decode(&self.read_custom_data(
            profile,
            CUSTOM_AREA_SIZE,
            FEATURE_METADATA_OFFSET,
            FEATURE_METADATA_SIZE,
        )?)
    }

    pub fn read_custom_area(&mut self, profile: u8) -> Result<Vec<u8>> {
        self.read_custom_data(profile, CUSTOM_AREA_SIZE, 0, CUSTOM_AREA_SIZE)
    }

    pub fn read_macro_bank(&mut self, profile: u8) -> Result<MacroBank> {
        validate_profile(profile)?;
        let base = KeyboardBase::decode(&self.read_base()?)?;
        let layout = MacroLayout::for_base(base.supports_multiple, base.supports_order);
        let offset = layout.offset(profile, base.max_profiles)?;
        MacroBank::decode(&self.read_range(0x0c, offset, layout.size)?)
    }

    pub fn apply_macro_bank(&mut self, profile: u8, value: &MacroBank) -> Result<MacroBank> {
        validate_profile(profile)?;
        value.encode()?;
        let mut checked_metadata = value.clone();
        checked_metadata.apply_local_metadata(&value.local_metadata())?;
        let base = KeyboardBase::decode(&self.read_base()?)?;
        let layout = MacroLayout::for_base(base.supports_multiple, base.supports_order);
        let offset = layout.offset(profile, base.max_profiles)?;
        ensure!(
            value.raw.len() == layout.size,
            "ACE75: банк макросов не соответствует текущей раскладке памяти"
        );
        let current = self.read_range(0x0c, offset, layout.size)?;
        let mut wanted = value.clone();
        wanted.raw = current.clone();
        let expected = wanted.encode()?;
        let timestamp = update_timestamp()?;
        let mut sync = Vec::new();
        if current != expected {
            let profiles: Vec<u8> = if layout.shared {
                (0..base.max_profiles).collect()
            } else {
                vec![profile]
            };
            for profile in profiles {
                let mut metadata = self.read_feature_metadata(profile)?;
                metadata.macro_updated_at = timestamp;
                sync.push((profile, metadata.encode()?));
            }
            self.write_range(0x0d, offset, &expected, 1)?;
        }
        let actual = self.read_range(0x0c, offset, layout.size)?;
        ensure!(actual == expected, "ACE75: проверка макросов не прошла");
        for (profile, metadata) in sync {
            self.write_custom_data_verified(
                profile,
                CUSTOM_AREA_SIZE,
                FEATURE_METADATA_OFFSET,
                &metadata,
            )?;
        }
        let mut actual = MacroBank::decode(&actual)?;
        // Names and editor delay preferences are local metadata, never HID bytes.
        actual.apply_local_metadata(&value.local_metadata())?;
        Ok(actual)
    }
}

fn update_timestamp() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("ACE75: системное время раньше UNIX epoch")?
        .as_millis()
        .try_into()
        .context("ACE75: отметка времени переполнена")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Memory {
        bank: Vec<u8>,
        custom: Vec<u8>,
        mutations: Vec<(u8, usize)>,
        replies: VecDeque<Vec<u8>>,
    }
    struct MemoryIo(Arc<Mutex<Memory>>);
    impl HidIo for MemoryIo {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            panic!("ACE75 has no feature reports")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            panic!("ACE75 has no feature reports")
        }
        fn write_output(&self, id: u8, packet: &[u8]) -> Result<()> {
            assert_eq!(id, 0);
            assert_eq!(packet[0], 0x55);
            let command = packet[1];
            let length = packet[4] as usize;
            let offset = u16::from_le_bytes([packet[5], packet[6]]) as usize;
            let mut memory = self.0.lock().unwrap();
            let data = match command {
                4 => include_bytes!("../../tests/fixtures/ace75/base-response.bin")
                    [8 + offset..8 + offset + length]
                    .to_vec(),
                0x0c => memory.bank[offset..offset + length].to_vec(),
                0xf1 => memory.custom[offset..offset + length].to_vec(),
                0x0d => {
                    memory.bank[offset..offset + length].copy_from_slice(&packet[8..8 + length]);
                    memory.mutations.push((command, offset));
                    vec![]
                }
                0xf2 => {
                    memory.custom[offset..offset + length].copy_from_slice(&packet[8..8 + length]);
                    memory.mutations.push((command, offset));
                    vec![]
                }
                _ => panic!("unexpected command {command:02x}"),
            };
            let mut reply = vec![0; 64];
            reply[..8].copy_from_slice(&[
                0xaa,
                command,
                0,
                0,
                data.len() as u8,
                offset as u8,
                (offset >> 8) as u8,
                0,
            ]);
            reply[8..8 + data.len()].copy_from_slice(&data);
            reply[3] = reply[4..8 + data.len()]
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
            memory.replies.push_back(reply);
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .replies
                .pop_front()
                .unwrap_or_default())
        }
    }
    fn memory() -> Arc<Mutex<Memory>> {
        let mut custom = vec![255; 4096];
        for profile in 0..4 {
            let offset = profile * 1024 + FEATURE_METADATA_OFFSET;
            let meta = br#"[1,2,"other_0","future"]"#;
            custom[offset..offset + meta.len()].copy_from_slice(meta);
        }
        Arc::new(Mutex::new(Memory {
            bank: vec![0; 8192],
            custom,
            mutations: vec![],
            replies: VecDeque::new(),
        }))
    }
    fn changed_bank() -> MacroBank {
        let mut bank = MacroBank::decode(&vec![0; 8192]).unwrap();
        bank.macros[0].name = "Тест".into();
        bank.macros[0].macro_actions = vec![
            MacroAction::Action {
                action: MacroKeyAction::Keydown,
                code: 4,
            },
            MacroAction::Time { delay: 10 },
            MacroAction::Action {
                action: MacroKeyAction::Keyup,
                code: 4,
            },
        ];
        bank
    }
    #[test]
    fn all_shared_metadata_is_preflighted_before_any_macro_mutation() {
        let memory = memory();
        memory.lock().unwrap().custom[2 * 1024 + FEATURE_METADATA_OFFSET] = b'{';
        let mut driver = Ace75::new(Box::new(MemoryIo(memory.clone())));
        assert!(driver.apply_macro_bank(0, &changed_bank()).is_err());
        assert!(memory.lock().unwrap().mutations.is_empty());
    }
    #[test]
    fn shared_macro_bank_written_once_and_all_metadata_preserves_foreign_lease() {
        let memory = memory();
        let mut driver = Ace75::new(Box::new(MemoryIo(memory.clone())));
        let observed = driver.apply_macro_bank(3, &changed_bank()).unwrap();
        assert_eq!(observed.macros[0].name, "Тест");
        assert_eq!(
            observed.macros[0].macro_actions,
            changed_bank().macros[0].macro_actions
        );
        let memory = memory.lock().unwrap();
        assert!(
            memory
                .mutations
                .iter()
                .filter(|(cmd, _)| *cmd == 0x0d)
                .all(|(_, offset)| *offset < 8192)
        );
        assert_eq!(
            memory
                .mutations
                .iter()
                .filter(|(cmd, _)| *cmd == 0xf2)
                .count(),
            4
        );
        let mut timestamps = Vec::new();
        for profile in 0..4 {
            let offset = profile * 1024 + FEATURE_METADATA_OFFSET;
            let meta = FeatureMetadata::decode(&memory.custom[offset..offset + 56]).unwrap();
            assert_eq!(meta.browser_id, "other");
            assert_eq!(meta.browser_id_expired_at, 0);
            assert_eq!(meta.profile_name_updated_at, 2);
            assert_eq!(meta.extra, [serde_json::json!("future")]);
            assert!(meta.macro_updated_at > 1);
            timestamps.push(meta.macro_updated_at);
        }
        assert!(timestamps.iter().all(|value| *value == timestamps[0]));
    }
    #[test]
    fn invalid_local_macro_delay_is_rejected_before_any_bank_write() {
        let memory = memory();
        let mut driver = Ace75::new(Box::new(MemoryIo(memory.clone())));
        let mut wanted = changed_bank();
        wanted.macros[0].default_delay = 4;
        assert!(driver.apply_macro_bank(0, &wanted).is_err());
        assert!(memory.lock().unwrap().mutations.is_empty());
        assert!(memory.lock().unwrap().bank.iter().all(|byte| *byte == 0));
    }
}
