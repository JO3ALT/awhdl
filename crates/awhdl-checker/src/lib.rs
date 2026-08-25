use awhdl_ast::{ConcurrentStatement, Declaration, Design, DesignUnit, SequentialStatement, Span};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

pub fn check(design: &Design) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut entities = BTreeMap::new();

    for unit in &design.units {
        if let DesignUnit::Entity(entity) = unit {
            if entities.insert(entity.name.as_str(), entity).is_some() {
                diagnostics.push(Diagnostic {
                    code: "AWHDL-E201",
                    message: format!("duplicate entity declaration: {}", entity.name),
                    span: entity.span,
                });
            }
            let mut port_names = BTreeSet::new();
            for port in &entity.ports {
                for name in &port.names {
                    if !port_names.insert(name) {
                        diagnostics.push(Diagnostic {
                            code: "AWHDL-E202",
                            message: format!("duplicate port declaration: {name}"),
                            span: port.span,
                        });
                    }
                }
            }
        }
    }

    for unit in &design.units {
        let DesignUnit::Architecture(architecture) = unit else {
            continue;
        };
        let Some(entity) = entities.get(architecture.entity.as_str()) else {
            diagnostics.push(Diagnostic {
                code: "AWHDL-E203",
                message: format!(
                    "architecture {} references unknown entity {}",
                    architecture.name, architecture.entity
                ),
                span: architecture.span,
            });
            continue;
        };

        let mut values = entity
            .ports
            .iter()
            .flat_map(|port| port.names.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut devices = BTreeSet::new();
        for declaration in &architecture.declarations {
            match declaration {
                Declaration::Device(device) => {
                    if !devices.insert(device.name.clone()) {
                        diagnostics.push(Diagnostic {
                            code: "AWHDL-E204",
                            message: format!("duplicate device declaration: {}", device.name),
                            span: device.span,
                        });
                    }
                }
                Declaration::Signal(signal) => {
                    for name in &signal.names {
                        if !values.insert(name.clone()) {
                            diagnostics.push(Diagnostic {
                                code: "AWHDL-E205",
                                message: format!("duplicate signal or port declaration: {name}"),
                                span: signal.span,
                            });
                        }
                    }
                }
            }
        }

        for statement in &architecture.statements {
            let ConcurrentStatement::Process(process) = statement;
            for trigger in &process.sensitivity {
                let root = trigger.split('.').next().unwrap_or(trigger);
                if !values.contains(root) && !devices.contains(root) {
                    diagnostics.push(Diagnostic {
                        code: "AWHDL-E206",
                        message: format!("unknown process sensitivity name: {trigger}"),
                        span: process.span,
                    });
                }
            }
            for sequential in &process.statements {
                match sequential {
                    SequentialStatement::DeviceCall(call) => {
                        if !devices.contains(&call.device) {
                            diagnostics.push(Diagnostic {
                                code: "AWHDL-E207",
                                message: format!("call references unknown device: {}", call.device),
                                span: call.span,
                            });
                        }
                        if !values.contains(&call.output) {
                            diagnostics.push(Diagnostic {
                                code: "AWHDL-E208",
                                message: format!(
                                    "call writes unknown signal or port: {}",
                                    call.output
                                ),
                                span: call.span,
                            });
                        }
                    }
                    SequentialStatement::Assignment(assignment) => {
                        if !values.contains(&assignment.target) {
                            diagnostics.push(Diagnostic {
                                code: "AWHDL-E209",
                                message: format!(
                                    "assignment writes unknown signal or port: {}",
                                    assignment.target
                                ),
                                span: assignment.span,
                            });
                        }
                    }
                    SequentialStatement::Null { .. } => {}
                }
            }
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use awhdl_parser::parse;

    #[test]
    fn accepts_the_minimal_hello_structure() {
        let source = include_str!("../../../examples/hello.awhdl");
        assert!(check(&parse(source).unwrap()).is_empty());
    }

    #[test]
    fn rejects_an_unknown_device_call() {
        let source = r#"
entity hello is
    port (task : in text<internal>; result : out text<internal>);
end hello;
architecture behavioral of hello is
begin
    process(task)
    begin
        missing.run(task) -> result;
    end process;
end behavioral;
"#;
        let diagnostics = check(&parse(source).unwrap());
        assert!(diagnostics.iter().any(|item| item.code == "AWHDL-E207"));
    }
}
