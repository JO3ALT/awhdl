use awhdl_ast::{
    ArchitectureDecl, AssertStmt, Assertion, Assignment, BarrierDecl, BinaryOp, BudgetDecl,
    BudgetLimit, BudgetValue, ConcurrentStatement, DataType, Declaration, Design, DesignUnit,
    DeviceCall, DeviceDecl, Duration, EntityDecl, Expr, Expression, Generic, GenericValue,
    IfBranch, IfStmt, ParallelStmt, PortDecl, PortMode, ProcessStmt, SequentialStatement,
    SignalDecl, Span, TimerDecl,
};
use pest::Parser;
use pest::error::{Error as PestError, LineColLocation};
use pest::iterators::{Pair, Pairs};
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

/// Children without keyword tokens, which the grammar keeps only for word boundaries.
fn items(pair: Pair<'_, Rule>) -> impl Iterator<Item = Pair<'_, Rule>> {
    pair.into_inner().filter(|item| !is_keyword(item.as_rule()))
}

fn is_keyword(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::kw_entity
            | Rule::kw_architecture
            | Rule::kw_device
            | Rule::kw_generic
            | Rule::kw_signal
            | Rule::kw_timer
            | Rule::kw_period
            | Rule::kw_budget
            | Rule::kw_barrier
            | Rule::kw_process
            | Rule::kw_port
            | Rule::kw_is
            | Rule::kw_of
            | Rule::kw_begin
            | Rule::kw_end
            | Rule::kw_if
            | Rule::kw_then
            | Rule::kw_elsif
            | Rule::kw_else
            | Rule::kw_parallel
            | Rule::kw_assert
            | Rule::kw_always
            | Rule::kw_never
            | Rule::kw_timeout
            | Rule::kw_on
            | Rule::kw_null
    )
}

fn parse_entity(pair: Pair<'_, Rule>) -> EntityDecl {
    let span = span(&pair);
    let mut inner = items(pair);
    let name = inner.next().expect("entity name").as_str().to_owned();
    let ports = inner
        .find(|item| item.as_rule() == Rule::port_clause)
        .map(parse_ports)
        .unwrap_or_default();
    EntityDecl { name, ports, span }
}

fn parse_ports(pair: Pair<'_, Rule>) -> Vec<PortDecl> {
    items(pair)
        .filter(|item| item.as_rule() == Rule::port_decl)
        .map(|item| {
            let item_span = span(&item);
            let mut inner = items(item);
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
    let mut inner = items(pair);
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
                    Rule::timer_decl => Declaration::Timer(parse_timer(declaration)),
                    Rule::budget_decl => Declaration::Budget(parse_budget(declaration)),
                    Rule::barrier_decl => Declaration::Barrier(parse_barrier(declaration)),
                    _ => unreachable!("grammar limits declarations"),
                });
            }
            Rule::concurrent_statement => {
                let statement = item.into_inner().next().expect("concurrent child");
                statements.push(match statement.as_rule() {
                    Rule::process_stmt => ConcurrentStatement::Process(parse_process(statement)),
                    Rule::concurrent_assert => {
                        ConcurrentStatement::Assert(parse_concurrent_assert(statement))
                    }
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
    let mut inner = items(pair);
    let name = inner.next().expect("device name").as_str().to_owned();
    let device_type = inner.next().expect("device type").as_str().to_owned();
    let generics = inner
        .next()
        .map(|clause| {
            items(clause)
                .map(|item| {
                    let item_span = span(&item);
                    let mut inner = items(item);
                    let key = inner.next().expect("generic key").as_str().to_owned();
                    let value = inner
                        .next()
                        .expect("generic value")
                        .into_inner()
                        .next()
                        .expect("generic value child");
                    Generic {
                        key,
                        value: match value.as_rule() {
                            Rule::string_literal => GenericValue::String(unquote(value.as_str())),
                            Rule::time_literal => GenericValue::Time(parse_time(value)),
                            Rule::integer => GenericValue::Integer(parse_integer(&value)),
                            Rule::ident => GenericValue::Ident(value.as_str().to_owned()),
                            _ => unreachable!("grammar limits generic values"),
                        },
                        span: item_span,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    DeviceDecl {
        name,
        device_type,
        generics,
        span: item_span,
    }
}

fn parse_signal(pair: Pair<'_, Rule>) -> SignalDecl {
    let item_span = span(&pair);
    let mut inner = items(pair);
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

fn parse_timer(pair: Pair<'_, Rule>) -> TimerDecl {
    let item_span = span(&pair);
    let mut inner = items(pair);
    TimerDecl {
        name: inner.next().expect("timer name").as_str().to_owned(),
        period: parse_time(inner.next().expect("timer period")),
        span: item_span,
    }
}

fn parse_budget(pair: Pair<'_, Rule>) -> BudgetDecl {
    let item_span = span(&pair);
    let mut inner = items(pair);
    let name = inner.next().expect("budget name").as_str().to_owned();
    let limits = inner
        .filter(|item| item.as_rule() == Rule::budget_limit)
        .map(|item| {
            let limit_span = span(&item);
            let mut inner = items(item);
            let key = inner.next().expect("budget key").as_str().to_owned();
            let value = inner.next().expect("budget value");
            BudgetLimit {
                key,
                value: match value.as_rule() {
                    Rule::time_literal => BudgetValue::Time(parse_time(value)),
                    _ => BudgetValue::Count(parse_integer(&value)),
                },
                span: limit_span,
            }
        })
        .collect();
    BudgetDecl {
        name,
        limits,
        span: item_span,
    }
}

fn parse_barrier(pair: Pair<'_, Rule>) -> BarrierDecl {
    let item_span = span(&pair);
    let mut inner = items(pair);
    BarrierDecl {
        name: inner.next().expect("barrier name").as_str().to_owned(),
        members: parse_ident_list(inner.next().expect("barrier members")),
        span: item_span,
    }
}

fn parse_concurrent_assert(pair: Pair<'_, Rule>) -> AssertStmt {
    let item_span = span(&pair);
    let form = items(pair).next().expect("assert form");
    let assertion = match form.as_rule() {
        Rule::assert_never_flow => {
            let mut inner = items(form);
            Assertion::NeverFlow {
                classification: inner.next().expect("classification").as_str().to_owned(),
                location: inner.next().expect("location").as_str().to_owned(),
            }
        }
        Rule::assert_always => Assertion::Always {
            condition: parse_expression(items(form).next().expect("condition")),
        },
        Rule::assert_never => Assertion::Never {
            condition: parse_expression(items(form).next().expect("condition")),
        },
        _ => unreachable!("grammar limits assertions"),
    };
    AssertStmt {
        assertion,
        span: item_span,
    }
}

fn parse_process(pair: Pair<'_, Rule>) -> ProcessStmt {
    let item_span = span(&pair);
    let mut sensitivity = Vec::new();
    let mut timeout = None;
    let mut statements = Vec::new();
    let mut on_timeout = Vec::new();
    for item in items(pair) {
        match item.as_rule() {
            Rule::name_list => {
                sensitivity.extend(item.into_inner().map(|name| name.as_str().to_owned()));
            }
            Rule::process_timeout => {
                timeout = Some(parse_time(items(item).next().expect("process timeout")));
            }
            Rule::sequential_statement => statements.push(parse_sequential(item)),
            Rule::on_timeout => on_timeout = parse_sequential_list(items(item)),
            _ => unreachable!("unexpected process child"),
        }
    }
    ProcessStmt {
        sensitivity,
        timeout,
        statements,
        on_timeout,
        span: item_span,
    }
}

fn parse_sequential_list<'a>(
    pairs: impl Iterator<Item = Pair<'a, Rule>>,
) -> Vec<SequentialStatement> {
    pairs
        .filter(|item| item.as_rule() == Rule::sequential_statement)
        .map(parse_sequential)
        .collect()
}

fn parse_sequential(pair: Pair<'_, Rule>) -> SequentialStatement {
    let statement = pair.into_inner().next().expect("sequential child");
    let statement_span = span(&statement);
    match statement.as_rule() {
        Rule::device_call => SequentialStatement::DeviceCall(parse_device_call(statement)),
        Rule::assignment => SequentialStatement::Assignment(parse_assignment(statement)),
        Rule::null_stmt => SequentialStatement::Null {
            span: statement_span,
        },
        Rule::assert_stmt => SequentialStatement::Assert {
            condition: parse_expression(items(statement).next().expect("assert condition")),
            span: statement_span,
        },
        Rule::parallel_stmt => SequentialStatement::Parallel(ParallelStmt {
            calls: items(statement).map(parse_device_call).collect(),
            span: statement_span,
        }),
        Rule::if_stmt => {
            let mut branches = Vec::new();
            let mut otherwise = Vec::new();
            let mut inner = items(statement).peekable();
            let condition = parse_expression(inner.next().expect("if condition"));
            let mut current = IfBranch {
                condition,
                statements: Vec::new(),
            };
            for item in inner {
                match item.as_rule() {
                    Rule::sequential_statement => current.statements.push(parse_sequential(item)),
                    Rule::elsif_clause => {
                        let mut clause = items(item);
                        let condition = parse_expression(clause.next().expect("elsif condition"));
                        branches.push(std::mem::replace(
                            &mut current,
                            IfBranch {
                                condition,
                                statements: parse_sequential_list(clause),
                            },
                        ));
                    }
                    Rule::else_clause => otherwise = parse_sequential_list(items(item)),
                    _ => unreachable!("unexpected if child"),
                }
            }
            branches.push(current);
            SequentialStatement::If(IfStmt {
                branches,
                otherwise,
                span: statement_span,
            })
        }
        _ => unreachable!("grammar limits sequential statements"),
    }
}

fn parse_device_call(pair: Pair<'_, Rule>) -> DeviceCall {
    let item_span = span(&pair);
    let mut inner = items(pair);
    let mut selected = inner.next().expect("selected device method").into_inner();
    let device = selected.next().expect("device name").as_str().to_owned();
    let method = selected.next().expect("method name").as_str().to_owned();
    let mut arguments = Vec::new();
    let mut timeout = None;
    let mut output = String::new();
    for item in inner {
        match item.as_rule() {
            Rule::argument_list => arguments = item.into_inner().map(parse_expression).collect(),
            Rule::call_timeout => {
                timeout = Some(parse_time(items(item).next().expect("call timeout")));
            }
            Rule::ident => output = item.as_str().to_owned(),
            _ => unreachable!("unexpected device call child"),
        }
    }
    DeviceCall {
        device,
        method,
        arguments,
        timeout,
        output,
        span: item_span,
    }
}

fn parse_assignment(pair: Pair<'_, Rule>) -> Assignment {
    let item_span = span(&pair);
    let mut inner = items(pair);
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
        expr: parse_expr(pair),
    }
}

fn parse_expr(pair: Pair<'_, Rule>) -> Expr {
    match pair.as_rule() {
        Rule::expression | Rule::primary => {
            parse_expr(pair.into_inner().next().expect("expression child"))
        }
        Rule::or_expr | Rule::and_expr | Rule::additive => fold_binary(pair.into_inner()),
        Rule::not_expr => {
            let mut inner = pair.into_inner();
            let first = inner.next().expect("not child");
            if first.as_rule() == Rule::op_not {
                Expr::Not {
                    operand: Box::new(parse_expr(inner.next().expect("not operand"))),
                }
            } else {
                parse_expr(first)
            }
        }
        Rule::relation => fold_binary(pair.into_inner()),
        Rule::string_literal => Expr::String {
            value: unquote(pair.as_str()),
        },
        Rule::integer => Expr::Integer {
            value: parse_integer(&pair),
        },
        Rule::time_literal => Expr::Time {
            value: parse_time(pair),
        },
        Rule::boolean => Expr::Bool {
            value: pair.as_str() == "true",
        },
        Rule::name => Expr::Name {
            path: pair
                .into_inner()
                .map(|part| part.as_str().to_owned())
                .collect(),
        },
        other => unreachable!("unexpected expression rule {other:?}"),
    }
}

/// Left-associative fold of `operand (operator operand)*`.
fn fold_binary(mut pairs: Pairs<'_, Rule>) -> Expr {
    let mut left = parse_expr(pairs.next().expect("left operand"));
    while let Some(operator) = pairs.next() {
        let op = match (operator.as_rule(), operator.as_str()) {
            (Rule::op_or, _) => BinaryOp::Or,
            (Rule::op_and, _) => BinaryOp::And,
            (_, "=") => BinaryOp::Eq,
            (_, "/=") => BinaryOp::Ne,
            (_, "<") => BinaryOp::Lt,
            (_, "<=") => BinaryOp::Le,
            (_, ">") => BinaryOp::Gt,
            (_, ">=") => BinaryOp::Ge,
            (_, "+") => BinaryOp::Add,
            (_, "-") => BinaryOp::Sub,
            (rule, text) => unreachable!("unexpected operator {rule:?} {text}"),
        };
        let right = parse_expr(pairs.next().expect("right operand"));
        left = Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    left
}

fn parse_time(pair: Pair<'_, Rule>) -> Duration {
    let mut inner = pair.into_inner();
    let amount = inner.next().expect("time amount").as_str();
    let unit = inner.next().expect("time unit").as_str();
    let scale = match unit {
        "ms" => 1,
        "sec" => 1_000,
        "min" => 60_000,
        "hour" => 3_600_000,
        "day" => 86_400_000,
        _ => unreachable!("grammar limits time units"),
    };
    // Negative or overflowing amounts saturate; the checker rejects non-positive times.
    let amount = amount.parse::<i64>().unwrap_or(i64::MAX).max(0) as u64;
    Duration {
        millis: amount.saturating_mul(scale),
    }
}

fn parse_integer(pair: &Pair<'_, Rule>) -> i64 {
    pair.as_str().parse().unwrap_or(i64::MAX)
}

fn unquote(literal: &str) -> String {
    literal[1..literal.len() - 1].replace("\\\"", "\"")
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

    #[test]
    fn selected_sensitivity_remains_an_untyped_name_and_event_declarations_are_unsupported() {
        for sensitivity in ["task.changed", "llm.completed"] {
            let design =
                parse(&HELLO.replace("process(task)", &format!("process({sensitivity})"))).unwrap();
            let DesignUnit::Architecture(architecture) = &design.units[1] else {
                panic!("architecture");
            };
            let ConcurrentStatement::Process(process) = &architecture.statements[0] else {
                panic!("process");
            };
            assert_eq!(process.sensitivity, vec![sensitivity.to_owned()]);
        }
        let with_event = HELLO.replace(
            "device llm : local_llm;",
            "device llm : local_llm;\n    event done : event<text>;",
        );
        assert!(parse(&with_event).is_err());
    }

    const V01: &str = r#"
entity review is
    port (
        task   : in  text<internal>;
        report : out text<internal>
    );
end review;

architecture flow of review is
    device coder : agent generic (location => local, clearance => restricted);
    device reviewer : agent generic (location => cloud, model => "example", clearance => internal);
    signal candidate : code<internal>;
    signal build_ok : flag<internal> := false;
    signal retries : count<internal> := 0;
    timer clock : period 10 min;
    budget loop_budget is
        iterations <= 10;
        wall_time <= 30 min;
    end budget;
    barrier checks (candidate, build_ok);
begin
    assert never (restricted -> cloud);
    assert always (retries <= 3);

    process(task) timeout 5 min;
    begin
        coder.run(task) timeout 2 min -> candidate;
        if retries >= 3 and not build_ok then
            null;
        elsif task = "stop" or retries + 1 > 2 then
            retries <= retries + 1;
        else
            parallel
                coder.build(candidate) -> build_ok;
                reviewer.review(candidate) -> report;
            end parallel;
        end if;
        assert retries /= 4;
    on timeout
        report <= "timed out";
    end process;

    process(clock, checks.ready)
    begin
        null;
    end process;
end flow;
"#;

    #[test]
    fn parses_v01_declarations_statements_and_expressions() {
        let design = parse(V01).unwrap();
        let DesignUnit::Architecture(architecture) = &design.units[1] else {
            panic!("architecture");
        };
        let kinds = architecture
            .declarations
            .iter()
            .map(|declaration| match declaration {
                Declaration::Device(_) => "device",
                Declaration::Signal(_) => "signal",
                Declaration::Timer(_) => "timer",
                Declaration::Budget(_) => "budget",
                Declaration::Barrier(_) => "barrier",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                "device", "device", "signal", "signal", "signal", "timer", "budget", "barrier"
            ]
        );
        let Declaration::Device(reviewer) = &architecture.declarations[1] else {
            panic!("device");
        };
        assert_eq!(
            reviewer.generic("location"),
            Some(&GenericValue::Ident("cloud".to_owned()))
        );
        assert_eq!(
            reviewer.generic("model"),
            Some(&GenericValue::String("example".to_owned()))
        );
        let Declaration::Timer(timer) = &architecture.declarations[5] else {
            panic!("timer");
        };
        assert_eq!(timer.period.millis, 600_000);
        let Declaration::Budget(budget) = &architecture.declarations[6] else {
            panic!("budget");
        };
        assert_eq!(
            budget.limits[1].value,
            BudgetValue::Time(Duration { millis: 1_800_000 })
        );
        assert!(matches!(
            architecture.statements[0],
            ConcurrentStatement::Assert(AssertStmt {
                assertion: Assertion::NeverFlow { .. },
                ..
            })
        ));
        let ConcurrentStatement::Process(process) = &architecture.statements[2] else {
            panic!("process");
        };
        assert_eq!(process.timeout, Some(Duration { millis: 300_000 }));
        assert_eq!(process.on_timeout.len(), 1);
        let SequentialStatement::DeviceCall(call) = &process.statements[0] else {
            panic!("call");
        };
        assert_eq!(call.timeout, Some(Duration { millis: 120_000 }));
        let SequentialStatement::If(branching) = &process.statements[1] else {
            panic!("if");
        };
        assert_eq!(branching.branches.len(), 2);
        assert!(matches!(
            branching.otherwise[0],
            SequentialStatement::Parallel(ParallelStmt { ref calls, .. }) if calls.len() == 2
        ));
        // `retries >= 3 and not build_ok`: and binds looser than comparison.
        let Expr::Binary { op, left, right } = &branching.branches[0].condition.expr else {
            panic!("binary");
        };
        assert_eq!(*op, BinaryOp::And);
        assert!(matches!(
            **left,
            Expr::Binary {
                op: BinaryOp::Ge,
                ..
            }
        ));
        assert!(matches!(**right, Expr::Not { .. }));
        // `task = "stop" or retries + 1 > 2`
        let Expr::Binary { op, right, .. } = &branching.branches[1].condition.expr else {
            panic!("binary");
        };
        assert_eq!(*op, BinaryOp::Or);
        let Expr::Binary { op, left, .. } = &**right else {
            panic!("comparison");
        };
        assert_eq!(*op, BinaryOp::Gt);
        assert!(matches!(
            **left,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
        let ConcurrentStatement::Process(timed) = &architecture.statements[3] else {
            panic!("process");
        };
        assert_eq!(timed.sensitivity, ["clock", "checks.ready"]);
    }

    #[test]
    fn keywords_need_word_boundaries_and_cannot_be_names() {
        // Identifiers that merely start with a keyword remain identifiers.
        let source = HELLO
            .replace("task", "index_input")
            .replace("llm", "endpoint_llm");
        assert!(parse(&source).is_ok());
        // A keyword cannot be used as a signal name.
        assert!(parse(&HELLO.replace("task", "then")).is_err());
        // Unsupported constructs stay syntax errors rather than being guessed.
        assert!(
            parse(&HELLO.replace("llm.run(task) -> result;", "await llm.run(task) -> result;"))
                .is_err()
        );
        assert!(
            parse(&HELLO.replace("llm.run(task) -> result;", "case task is end case;")).is_err()
        );
    }
}
