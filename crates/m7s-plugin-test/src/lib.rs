//! Built-in self-test registry.
#[derive(Default)]
pub struct SelfTest { checks: Vec<(String, bool)> }
impl SelfTest {
    pub fn check(&mut self, name: &str, ok: bool) { self.checks.push((name.to_string(), ok)); }
    pub fn pass(&self) -> bool { self.checks.iter().all(|(_, ok)| *ok) }
    pub fn summary(&self) -> String { format!("{}/{} checks passed", self.checks.iter().filter(|(_,ok)| *ok).count(), self.checks.len()) }
}
#[cfg(test)] mod tests { use super::*; #[test] fn detects_fail() { let mut t=SelfTest::default(); t.check("a",true); t.check("b",false); assert!(!t.pass()); } }
