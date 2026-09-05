# Model and XML verification map

| Surface | Portable evidence | Native evidence |
| --- | --- | --- |
| Exec, COM handler, Email, ShowMessage | `all_known_actions_preserve_fields`, UTF-8/UTF-16 round trips | Exec lifecycle; handler release DLL fixture |
| Boot, Registration, Idle, Time, Event, Logon, Session, Daily, Weekly, Monthly, MonthlyDow | `all_known_trigger_families_preserve_common_and_specific_fields` | Registration/readback smoke; actual firing depends on trigger/environment |
| TaskSettings fields and schema 1.2–1.6 | `settings_and_schema_versions_round_trip`, older-schema rejection | Native validation and registration |
| Principal/logon combinations | `logon_identities_preserve_semantics`, mismatch validation | Current-user registration passed; SYSTEM fixture requires an elevated host; remote/password cases need dedicated credentials |
| Unknown extensions and raw snapshots | Existing extension/raw fixtures; canonical-output fuzzing | Unknown schemas remain raw and are not blindly reconciled |
| Escapes, character references, Unicode | Fixed corpus plus 512 generated round-trip cases | Existing Windows tasks containing XML escapes |
| DTD, illegal entities, depth/size limits | Boundary rejection and fuzz target | Native event XML is subject to the same bounded parser |
| Dates, durations, cron | Existing schedule/time tests and input fuzzing | Windows owns DST and actual scheduling semantics |

These rows distinguish model representation from native execution. A round-trip
test does not prove that every Windows version supports or fires a trigger.
Failure reproducers belong alongside the production parser/state-machine tests.
