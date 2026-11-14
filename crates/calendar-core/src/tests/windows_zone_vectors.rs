use super::{assert_no_failures, vector_cases};
use crate::windows_time_zones::WindowsTimeZones;

use super::support::zone_or_none;

const WINDOWS_NAMES: &str = include_str!("../../../../vectors/zones/windows-names.json");

#[test]
fn windows_names_vector() {
    let cases = vector_cases(WINDOWS_NAMES);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let input = &case["input"];
        let expected = &case["expected"];
        match input["fn"].as_str().unwrap() {
            "timeZone" => {
                let windows_name = input["name"].as_str().unwrap();
                let actual = WindowsTimeZones::time_zone_for(windows_name)
                    .map(|zone| zone.identifier().to_string());
                let want = expected["identifier"].as_str().map(str::to_string);
                if actual != want {
                    failures.push(format!(
                        "{name}: {windows_name:?} gave {actual:?}, expected {want:?}"
                    ));
                }
            }
            "windowsName" => {
                let id = input["zone"].as_str().unwrap();
                let want = expected["name"].as_str().map(str::to_string);
                match zone_or_none(id) {
                    None => failures.push(format!("{name}: Zone::named({id:?}) is None")),
                    Some(zone) => {
                        let actual = WindowsTimeZones::windows_name_for(&zone);
                        if actual != want {
                            failures
                                .push(format!("{name}: {id:?} gave {actual:?}, expected {want:?}"));
                        }
                    }
                }
            }
            other => failures.push(format!("{name}: unknown fn {other}")),
        }
    }
    assert_eq!(cases.len(), 478);
    assert_no_failures("zones/windows-names", cases.len(), failures);
}
