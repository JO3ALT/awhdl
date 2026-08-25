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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDecl {
    pub name: String,
    pub device_type: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDecl {
    pub names: Vec<String>,
    pub data_type: DataType,
    pub initial: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConcurrentStatement {
    Process(ProcessStmt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessStmt {
    pub sensitivity: Vec<String>,
    pub statements: Vec<SequentialStatement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SequentialStatement {
    DeviceCall(DeviceCall),
    Assignment(Assignment),
    Null { span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCall {
    pub device: String,
    pub method: String,
    pub arguments: Vec<Expression>,
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
pub struct Expression {
    pub source: String,
    pub span: Span,
}
