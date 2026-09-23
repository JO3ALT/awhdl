use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Design {
    pub units: Vec<DesignUnit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesignUnit {
    Entity(EntityDecl),
    Architecture(ArchitectureDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityDecl {
    pub name: String,
    pub ports: Vec<PortDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDecl {
    pub names: Vec<String>,
    pub mode: PortMode,
    pub data_type: DataType,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortMode {
    In,
    Out,
    Inout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataType {
    pub name: String,
    pub classification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureDecl {
    pub name: String,
    pub entity: String,
    pub declarations: Vec<Declaration>,
    pub statements: Vec<ConcurrentStatement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Declaration {
    Device(DeviceDecl),
    Signal(SignalDecl),
    Timer(TimerDecl),
    Budget(BudgetDecl),
    Barrier(BarrierDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDecl {
    pub name: String,
    pub device_type: String,
    /// `generic (key => value, ...)`, in source order.
    pub generics: Vec<Generic>,
    pub span: Span,
}

impl DeviceDecl {
    pub fn generic(&self, key: &str) -> Option<&GenericValue> {
        self.generics
            .iter()
            .find(|generic| generic.key == key)
            .map(|generic| &generic.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generic {
    pub key: String,
    pub value: GenericValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GenericValue {
    Ident(String),
    String(String),
    Integer(i64),
    Time(Duration),
}

impl GenericValue {
    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Self::Ident(value) => Some(value),
            _ => None,
        }
    }
}

/// A time literal normalized to milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Duration {
    pub millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDecl {
    pub names: Vec<String>,
    pub data_type: DataType,
    pub initial: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerDecl {
    pub name: String,
    pub period: Duration,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetDecl {
    pub name: String,
    pub limits: Vec<BudgetLimit>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimit {
    pub key: String,
    pub value: BudgetValue,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BudgetValue {
    Count(i64),
    Time(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarrierDecl {
    pub name: String,
    pub members: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConcurrentStatement {
    Process(ProcessStmt),
    Assert(AssertStmt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessStmt {
    pub sensitivity: Vec<String>,
    /// Process-level deadline; `on_timeout` runs when it expires.
    pub timeout: Option<Duration>,
    pub statements: Vec<SequentialStatement>,
    pub on_timeout: Vec<SequentialStatement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertStmt {
    pub assertion: Assertion,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assertion {
    /// `assert always (expr);`: must hold whenever evaluated.
    Always { condition: Expression },
    /// `assert never (expr);`
    Never { condition: Expression },
    /// `assert never (restricted -> cloud);`: a static information-flow rule.
    NeverFlow {
        classification: String,
        location: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SequentialStatement {
    DeviceCall(DeviceCall),
    Assignment(Assignment),
    If(IfStmt),
    Parallel(ParallelStmt),
    Assert { condition: Expression, span: Span },
    Null { span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCall {
    pub device: String,
    pub method: String,
    pub arguments: Vec<Expression>,
    /// Operation-level `timeout <time>`.
    pub timeout: Option<Duration>,
    pub output: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub target: String,
    pub value: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfStmt {
    /// `if` followed by any `elsif` branches, in order.
    pub branches: Vec<IfBranch>,
    pub otherwise: Vec<SequentialStatement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfBranch {
    pub condition: Expression,
    pub statements: Vec<SequentialStatement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelStmt {
    pub calls: Vec<DeviceCall>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expression {
    pub source: String,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    String {
        value: String,
    },
    Integer {
        value: i64,
    },
    Bool {
        value: bool,
    },
    Time {
        value: Duration,
    },
    /// A signal, port, or selected name such as `test.done` or `build.ok`.
    Name {
        path: Vec<String>,
    },
    Not {
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
}

impl Expr {
    /// Every name referenced by the expression, for scope and flow checks.
    pub fn names(&self) -> Vec<&[String]> {
        let mut names = Vec::new();
        self.collect_names(&mut names);
        names
    }

    fn collect_names<'a>(&'a self, names: &mut Vec<&'a [String]>) {
        match self {
            Self::Name { path } => names.push(path),
            Self::Not { operand } => operand.collect_names(names),
            Self::Binary { left, right, .. } => {
                left.collect_names(names);
                right.collect_names(names);
            }
            _ => {}
        }
    }
}
