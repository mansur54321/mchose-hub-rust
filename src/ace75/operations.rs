use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeyboardNotification {
    Calibration {
        key: KeyAssignment,
        travel: u16,
        calibration_state: u8,
    },
    ActiveProfile(u8),
    ProfileChanging(u8),
    Reset,
    FunctionsChanged,
    LightFrame(Vec<u8>),
    Unknown(Vec<u8>),
}

pub fn decode_notification(raw: &[u8]) -> Result<KeyboardNotification> {
    ensure!(!raw.is_empty(), "ACE75: пустое уведомление");
    Ok(match raw[0] {
        0xa0 => {
            ensure!(raw.len() >= 11, "ACE75: обрезанная телеметрия калибровки");
            KeyboardNotification::Calibration {
                key: KeyAssignment {
                    kind: raw[1],
                    code1: raw[2],
                    code2: raw[3],
                },
                travel: u16::from_be_bytes([raw[6], raw[7]]),
                calibration_state: raw[10],
            }
        }
        0xa1 | 0xa4 => {
            ensure!(
                raw.len() >= 3 && raw[2] < PROFILE_COUNT as u8,
                "ACE75: неверное уведомление профиля"
            );
            if raw[0] == 0xa1 {
                KeyboardNotification::ActiveProfile(raw[2])
            } else {
                KeyboardNotification::ProfileChanging(raw[2])
            }
        }
        0xa2 => KeyboardNotification::Reset,
        0xa3 => KeyboardNotification::FunctionsChanged,
        0xaa if raw.get(1) == Some(&0xfa) => KeyboardNotification::LightFrame(raw.to_vec()),
        _ => KeyboardNotification::Unknown(raw.to_vec()),
    })
}

fn decode_colors(raw: &[u8]) -> Result<Vec<[u8; 3]>> {
    ensure!(raw.len() % 3 == 0, "ACE75: обрезанная таблица RGB");
    Ok(raw
        .chunks_exact(3)
        .map(|value| [value[0], value[1], value[2]])
        .collect())
}

pub fn music_requests(profile: u8, scope: LightScope, colors: &[[u8; 3]]) -> Result<Vec<Request>> {
    validate_profile(profile)?;
    ensure!(
        colors.len() == KEY_COUNT,
        "ACE75: кадр подсветки должен содержать 128 RGB-позиций"
    );
    let (command, ranges): (u8, &[(usize, usize)]) = match scope {
        LightScope::Main => (0xdd, &[(0, 95), (124, 128)]),
        LightScope::Side => (0xdf, &[(95, 123)]),
        LightScope::Side2 => (0xdc, &[(123, 124)]),
    };
    let mut requests = Vec::new();
    for &(start, end) in ranges {
        let bytes: Vec<u8> = colors[start..end].iter().flatten().copied().collect();
        requests.extend(write_requests(
            command,
            profile as usize * 512 + start * 3,
            &bytes,
            3,
        )?);
    }
    Ok(requests)
}

impl Ace75 {
    pub fn read_colors(&mut self, profile: u8) -> Result<Vec<[u8; 3]>> {
        validate_profile(profile)?;
        decode_colors(&self.read_range(0x0a, profile as usize * 512, KEY_COUNT * 3)?)
    }

    pub fn read_realtime_colors(&mut self) -> Result<Vec<[u8; 3]>> {
        decode_colors(&self.read_range(0xde, 0, KEY_COUNT * 3)?)
    }

    pub fn apply_colors(
        &mut self,
        profile: u8,
        first_index: usize,
        colors: &[[u8; 3]],
    ) -> Result<Vec<[u8; 3]>> {
        validate_profile(profile)?;
        ensure!(
            !colors.is_empty()
                && first_index
                    .checked_add(colors.len())
                    .is_some_and(|end| end <= KEY_COUNT),
            "ACE75: цветовая область выходит за 128 позиций"
        );
        let expected: Vec<u8> = colors.iter().flatten().copied().collect();
        let offset = profile as usize * 512 + first_index * 3;
        self.write_range(0x0b, offset, &expected, 3)?;
        let actual = self.read_range(0x0a, offset, expected.len())?;
        ensure!(
            actual == expected,
            "ACE75: проверка цветов клавиш не прошла"
        );
        decode_colors(&actual)
    }

    pub fn stream_colors(
        &mut self,
        profile: u8,
        scope: LightScope,
        colors: &[[u8; 3]],
    ) -> Result<()> {
        let requests = music_requests(profile, scope, colors)?;
        for request in requests {
            self.io.write_output(REPORT_ID, &request.packet)?;
            // The vendor treats these transient frames as complete after 20ms,
            // including firmware that does not acknowledge music packets.
            let deadline = Instant::now() + Duration::from_millis(20);
            for _ in 0..32 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let raw = self
                    .io
                    .read_input(remaining.as_millis().clamp(1, 20) as i32)?;
                if raw.is_empty() {
                    break;
                }
                if raw.len() >= 8
                    && raw[0] == 0xaa
                    && raw[1] == request.command
                    && u16::from_le_bytes([raw[5], raw[6]]) == request.offset
                {
                    let response = decode_response(&raw)?;
                    ensure!(
                        response.payload.len() <= request.length as usize,
                        "ACE75: неверный ACK музыкального кадра"
                    );
                    break;
                }
                self.preserve_notification(raw);
            }
        }
        Ok(())
    }

    pub fn read_notification(&mut self, timeout_ms: i32) -> Result<Option<KeyboardNotification>> {
        ensure!(
            (0..=1000).contains(&timeout_ms),
            "ACE75: недопустимый таймаут телеметрии"
        );
        let raw = if let Some(raw) = self.notifications.pop_front() {
            raw
        } else {
            self.io.read_input(timeout_ms)?
        };
        if raw.is_empty() {
            return Ok(None);
        }
        Ok(Some(decode_notification(&raw)?))
    }

    pub fn set_debug_mode(&mut self, profile: u8, enabled: bool) -> Result<FunctionConfig> {
        validate_profile(profile)?;
        let current = FunctionConfig::decode(&self.read_function_config(profile)?)?;
        let mut value = current.performance.clone();
        value.debug_mode = enabled;
        let expected = current.with_performance(&value, false)?;
        self.write_functions_verified(profile, &expected)
    }

    pub fn start_calibration(&mut self) -> Result<()> {
        self.write_range(0xa8, 0, &[0], 1)
    }
    pub fn end_calibration(&mut self) -> Result<()> {
        self.write_range(0xa9, 0, &[0], 1)
    }
    pub fn restart_calibration(&mut self) -> Result<()> {
        self.end_calibration()?;
        self.write_range(0xb1, 0, &[0], 1)?;
        self.start_calibration()
    }
    pub fn reset(&mut self, profile: Option<u8>) -> Result<()> {
        if let Some(profile) = profile {
            validate_profile(profile)?;
        }
        self.write_range(0xee, 0, &[profile.unwrap_or(0xff)], 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_uses_big_endian_travel_and_preserves_key_identity() {
        let raw = [0xa0, 16, 0, 4, 0, 0, 1, 44, 0, 0, 7];
        assert_eq!(
            decode_notification(&raw).unwrap(),
            KeyboardNotification::Calibration {
                key: KeyAssignment {
                    kind: 16,
                    code1: 0,
                    code2: 4
                },
                travel: 300,
                calibration_state: 7
            }
        );
        assert!(decode_notification(&raw[..10]).is_err());
        assert_eq!(
            decode_notification(&[0xa1, 0, 3]).unwrap(),
            KeyboardNotification::ActiveProfile(3)
        );
        assert!(decode_notification(&[0xa1, 0, 4]).is_err());
    }
    #[test]
    fn music_commands_cannot_cross_into_another_lighting_zone() {
        let colors: Vec<_> = (0..128).map(|index| [index as u8, 2, 3]).collect();
        let main = music_requests(2, LightScope::Main, &colors).unwrap();
        assert!(main.iter().all(|packet| packet.command == 0xdd));
        for packet in &main {
            let offset = packet.offset as usize - 1024;
            assert!(offset + packet.length as usize <= 95 * 3 || offset >= 124 * 3);
        }
        let side = music_requests(2, LightScope::Side, &colors).unwrap();
        assert_eq!(side.len(), 2);
        assert_eq!(side[0].offset, 1024 + 95 * 3);
        assert_eq!(side[1].offset, 1024 + 123 * 3 - 54);
        let logo = music_requests(2, LightScope::Side2, &colors).unwrap();
        assert_eq!(logo.len(), 1);
        assert_eq!(logo[0].command, 0xdc);
        assert_eq!(logo[0].offset, 1024 + 123 * 3);
        assert_eq!(&logo[0].packet[8..11], &colors[123]);
    }
    #[test]
    fn music_zone_packets_match_original_sdk_with_117_capabilities() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/ace75/sdk-golden-vectors.json"
        ))
        .unwrap();
        for case in fixture["musicVectors"].as_array().unwrap() {
            let scope = match case["scope"].as_str().unwrap() {
                "light" => LightScope::Main,
                "sideLight" => LightScope::Side,
                "sideLight2" => LightScope::Side2,
                _ => panic!(),
            };
            let colors: Vec<[u8; 3]> = serde_json::from_value(case["colors"].clone()).unwrap();
            let actual = music_requests(2, scope, &colors).unwrap();
            let expected = case["packets"].as_array().unwrap();
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                let expected: Vec<u8> = serde_json::from_value(expected.clone()).unwrap();
                assert_eq!(actual.packet.as_slice(), expected);
            }
        }
    }
}
