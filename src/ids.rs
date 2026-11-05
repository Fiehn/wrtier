
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChangeId {
    pub counter: u32,
    pub device: DeviceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharId {
    pub change: ChangeId,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hlc(pub u64); // Milliseconds since 1970

impl Hlc {
    pub fn tick(&mut self, now_ms: u64) -> Hlc {
        if now_ms > self.0 {
            self.0 = now_ms;
        } else {
            self.0 += 1;
        }
        *self
    }
    pub fn observe(&mut self, seen: Hlc) {
        if seen.0 > self.0 {
            self.0 = seen.0;
        }
    }
}

impl std::fmt::Display for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}", self.0) // DeviceId(42) -> "0000002a"
    }
}
impl std::fmt::Display for ChangeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.device, self.counter)
    }
}
impl std::fmt::Display for CharId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}+{}", self.change, self.offset) // "0000002a:7+3"
    }
}

impl std::str::FromStr for DeviceId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n = u32::from_str_radix(s, 16)?; // "0000002a" -> 42
        Ok(DeviceId(n))
    }
}
impl std::str::FromStr for ChangeId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid ChangeId format: {}", s));
        }
        let device = parts[0].parse::<DeviceId>().map_err(|e| e.to_string())?;
        let counter = parts[1].parse::<u32>().map_err(|e| e.to_string())?;
        Ok(ChangeId { device, counter })
    }
}
impl std::str::FromStr for CharId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (change, offset) = s
            .split_once('+')
            .ok_or_else(|| format!("Invalid CharId format: {}", s))?;
        let change = change.parse::<ChangeId>()?;
        let offset = offset.parse::<u32>().map_err(|e| e.to_string())?;
        Ok(CharId { change, offset })
    }
}
impl std::str::FromStr for Hlc {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let n = s.parse::<u64>()?;
        Ok(Hlc(n))
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    // 1. CharId -> string -> CharId gives back the same value
    #[test]
    fn test_char_id_string_roundtrip() {
        let c = CharId {
            change: ChangeId {
                device: DeviceId(42),
                counter: 7,
            },
            offset: 3,
        };
        let s = c.to_string();
        let c2 = s.parse::<CharId>().unwrap();
        assert_eq!(c, c2);

    }

    // 2. Hlc::tick never goes backwards, even if now_ms goes backwards
    #[test]
    fn test_hlc_tick_never_goes_backwards() {
        let mut hlc = Hlc(1000);
        hlc.tick(2000);
        assert_eq!(hlc.0, 2000);
        hlc.tick(1500);
        assert_eq!(hlc.0, 2001);
    }

    // 3. after observe(Hlc(5000)), tick(1000) returns more than 5000
    #[test]
    fn test_hlc_observe_and_tick() {
        let mut hlc = Hlc(4000);
        hlc.observe(Hlc(5000));
        hlc.tick(1000);
        assert!(hlc.0 > 5000);
    }

    // Ensure formatting of DeviceId, ChangeId, CharId, and Hlc is correct
    #[test]
    fn test_char_id_text_form() {
        let c = CharId {
            change: ChangeId { device: DeviceId(42), counter: 7 },
            offset: 3,
        };
        assert_eq!(c.to_string(), "0000002a:7+3");
    }

}