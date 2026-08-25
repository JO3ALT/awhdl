use awhdl_ast::{
    ArchitectureDecl, Assignment, ConcurrentStatement, DataType, Declaration, Design, DesignUnit,
    DeviceCall, DeviceDecl, EntityDecl, Expression, PortDecl, PortMode, ProcessStmt,
    SequentialStatement, SignalDecl, Span,
};
use pest::Parser;
use pest::error::{Error as PestError, LineColLocation};
use pest::iterators::Pair;
use pest_derive::Parser;
use thiserror::Error;

#[derive(Parser)]
#[grammar = "awhdl.pest"]
struct AwhdlParser;

#[derive(Debug, Error)]
#[error("AWHDL-E101 at {line}:{column}: {message}")]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl From<PestError<Rule>> for ParseError {
    fn from(error: PestError<Rule>) -> Self {
        let (line, column) = match error.line_col {
            LineColLocation::Pos(position) => position,
            LineColLocation::Span(start, _) => start,
        };
        Self {
            message: error.variant.message().into_owned(),
            line,
            column,
        }
    }
}

pub fn parse(source: &str) -> Result<Design, ParseError> {
    let mut pairs = AwhdlParser::parse(Rule::program, source)?;
    let program = pairs.next().expect("program rule always yields one pair");
    let mut units = Vec::new();
    for pair in program.into_inner() {
        if pair.as_rule() != Rule::design_unit {
            continue;
        }
        let unit = pair.into_inner().next().expect("design unit has a child");
        units.push(match unit.as_rule() {
            Rule::entity_decl => DesignUnit::Entity(parse_entity(unit)),
            Rule::architecture_decl => DesignUnit::Architecture(parse_architecture(unit)),
            _ => unreachable!("grammar limits design units"),
        });
    }
    Ok(Design { units })
}

fn parse_entity(pair: Pair<'_, Rule>) -> EntityDecl {
    let span = span(&pair);
    let mut inner = pair.into_inner();
    let name = inner.next().expect("entity name").as_str().to_owned();
    let ports = inner
        .find(|item| item.as_rule() == Rule::port_clause)
        .map(parse_ports)
        .unwrap_or_default();
    EntityDecl { name, ports, span }
}

fn parse_ports(pair: Pair<'_, Rule>) -> Vec<PortDecl> {
    pair.into_inner()
        .filter(|item| item.as_rule() == Rule::port_decl)
        .map(|item| {
            let item_span = span(&item);
            let mut inner = item.into_inner();
            let names = parse_ident_list(inner.next().expect("port names"));
            let mode = match inner.next().expect("port mode").as_str() {
                "in" => PortMode::In,
                "out" => PortMode::Out,
                "inout" => PortMode::Inout,
                _ => unreachable!("grammar limits port modes"),
            };
            let data_type = parse_data_type(inner.next().expect("port type"));
            PortDecl {
                names,
                mode,
                data_type,
                span: item_span,
            }
        })
        .collect()
}

fn parse_architecture(pair: Pair<'_, Rule>) -> ArchitectureDecl {
    let architecture_span = span(&pair);
    let mut inner = pair.into_inner();
    let name = inner.next().expect("architecture name").as_str().to_owned();
    let entity = inner
        .next()
        .expect("architecture entity")
        .as_str()
        .to_owned();
    let mut declarations = Vec::new();
    let mut statements = Vec::new();
    for item in inner {
        match item.as_rule() {
            Rule::declaration => {
                let declaration = item.into_inner().next().expect("declaration child");
                declarations.push(match declaration.as_rule() {
                    Rule::device_decl => Declaration::Device(parse_device(declaration)),
                    Rule::signal_decl => Declaration::Signal(parse_signal(declaration)),
                    _ => unreachable!("grammar limits declarations"),
                });
            }
            Rule::concurrent_statement => {
                let statement = item.into_inner().next().expect("concurrent child");
                statements.push(match statement.as_rule() {
                    Rule::process_stmt => ConcurrentStatement::Process(parse_process(statement)),
                    _ => unreachable!("grammar limits concurrent statements"),
                });
            }
            Rule::ident => {}
            _ => unreachable!("unexpected architecture child"),
        }
    }
    ArchitectureDecl {
        name,
        entity,
        declarations,
        statements,
        span: architecture_span,
    }
}

fn parse_device(pair: Pair<'_, Rule>) -> DeviceDecl {
    let item_span = span(&pair);
    let mut inner = pair.into_inner();
    DeviceDecl {
        name: inner.next().expect("device name").as_str().to_owned(),
        device_type: inner.next().expect("device type").as_str().to_owned(),
        span: item_span,
    }
}

fn parse_signal(pair: Pair<'_, Rule>) -> SignalDecl {
    let item_span = span(&pair);
    let mut inner = pair.into_inner();
    let names = parse_ident_list(inner.next().expect("signal names"));
    let data_type = parse_data_type(inner.next().expect("signal type"));
    let initial = inner.next().map(parse_expression);
    SignalDecl {
        names,
        data_type,
        initial,
        span: item_span,
    }
}

fn parse_process(pair: Pair<'_, Rule>) -> ProcessStmt {
    let item_span = span(&pair);
    let mut sensitivity = Vec::new();
    let mut statements = Vec::new();
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::name_list => {
                sensitivity.extend(item.into_inner().map(|name| name.as_str().to_owned()));
            }
            Rule::sequential_statement => {
                let statement = item.into_inner().next().expect("sequential child");
                statements.push(match statement.as_rule() {
                    Rule::device_call => {
                        SequentialStatement::DeviceCall(parse_device_call(statement))
                    }
                    Rule::assignment => {
                        SequentialStatement::Assignment(parse_assignment(statement))
                    }
                    Rule::null_stmt => SequentialStatement::Null {
                        span: span(&statement),
                    },
                    _ => unreachable!("grammar limits sequential statements"),
                });
            }
            _ => unreachable!("unexpected process child"),
        }
    }
    ProcessStmt {
        sensitivity,
        statements,
        span: item_span,
    }
}

fn parse_device_call(pair: Pair<'_, Rule>) -> DeviceCall {
    let item_span = span(&pair);
    let mut inner = pair.into_inner();
    let selected = inner.next().expect("selected device method");
    let mut selected_inner = selected.into_inner();
    let device = selected_inner
        .next()
        .expect("device name")
        .as_str()
        .to_owned();
    let method = selected_inner
        .next()
        .expect("method name")
        .as_str()
        .to_owned();
    let next = inner.next().expect("arguments or output");
    let (arguments, output_pair) = if next.as_rule() == Rule::argument_list {
        (
            next.into_inner().map(parse_expression).collect(),
            inner.next().expect("call output"),
        )
    } else {
        (Vec::new(), next)
    };
    DeviceCall {
        device,
        method,
        arguments,
        output: output_pair.as_str().to_owned(),
        span: item_span,
    }
}

fn parse_assignment(pair: Pair<'_, Rule>) -> Assignment {
    let item_span = span(&pair);
    let mut inner = pair.into_inner();
    Assignment {
        target: inner.next().expect("assignment target").as_str().to_owned(),
        value: parse_expression(inner.next().expect("assignment value")),
        span: item_span,
    }
}

fn parse_expression(pair: Pair<'_, Rule>) -> Expression {
    Expression {
        source: pair.as_str().to_owned(),
        span: span(&pair),
    }
}

fn parse_ident_list(pair: Pair<'_, Rule>) -> Vec<String> {
    pair.into_inner()
        .map(|item| item.as_str().to_owned())
        .collect()
}

fn parse_data_type(pair: Pair<'_, Rule>) -> DataType {
    let mut inner = pair.into_inner();
    DataType {
        name: inner.next().expect("type name").as_str().to_owned(),
        classification: inner.next().map(|item| item.as_str().to_owned()),
    }
}

fn span(pair: &Pair<'_, Rule>) -> Span {
    let pair_span = pair.as_span();
    Span {
        start: pair_span.start(),
        end: pair_span.end(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = r#"
entity hello is
    port (
        task   : in  text<internal>;
        result : out text<internal>
    );
end hello;

architecture behavioral of hello is
    device llm : local_llm;
begin
    process(task)
    begin
        llm.run(task) -> result;
    end process;
end behavioral;
"#;

    #[test]
    fn parses_minimal_hello_design() {
        let design = parse(HELLO).unwrap();
        assert_eq!(design.units.len(), 2);
        let DesignUnit::Entity(entity) = &design.units[0] else {
            panic!("first unit is not an entity");
        };
        assert_eq!(entity.name, "hello");
        assert_eq!(entity.ports.len(), 2);
        let DesignUnit::Architecture(architecture) = &design.units[1] else {
            panic!("second unit is not an architecture");
        };
        assert_eq!(architecture.entity, "hello");
        assert_eq!(architecture.statements.len(), 1);
    }

    #[test]
    fn syntax_error_reports_line_and_column() {
        let error = parse("entity broken is\nend broken").unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.column > 0);
    }
}
