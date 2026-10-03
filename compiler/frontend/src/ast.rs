use diagnostics::{Diagnostic, Span};

#[derive(Clone, Debug)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

pub fn sp<T>(node: T, span: Span) -> Spanned<T> {
    Spanned { node, span }
}

#[derive(Clone, Debug)]
pub struct Module {
    pub inner_attrs: Vec<Attr>,
    pub decls: Vec<Spanned<Decl>>,
    pub docs: String,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub inner: bool,
    pub name: String,
    pub args: Vec<Spanned<Expr>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visibility {
    Export,
    Internal,
    Private,
}

pub type Access = Visibility;

#[derive(Clone, Debug)]
pub enum Decl {
    Class {
        access: Access,
        name: String,
        type_params: Vec<String>,
        extends: Option<Type>,
        with: Vec<Type>,
        members: Vec<Spanned<ClassMember>>,
        docs: String,
    },
    Struct {
        access: Access,
        name: String,
        type_params: Vec<String>,
        members: Vec<Spanned<ClassMember>>,
        docs: String,
    },
    Trait {
        access: Access,
        name: String,
        members: Vec<Spanned<ClassMember>>,
        docs: String,
    },
    Interface {
        access: Access,
        name: String,
        type_params: Vec<String>,
        members: Vec<Spanned<ClassMember>>,
        docs: String,
    },
    Extension {
        access: Access,
        target: Type,
        members: Vec<Spanned<ClassMember>>,
        docs: String,
    },
    Enum {
        access: Access,
        name: String,
        type_params: Vec<String>,
        members: Vec<EnumMember>,
        docs: String,
    },
    Record {
        access: Access,
        name: String,
        type_params: Vec<String>,
        fields: Vec<RecordField>,
        docs: String,
    },
    Fn(FnDecl),
    Const {
        access: Access,
        name: String,
        ty: Option<Type>,
        value: Spanned<Expr>,
        docs: String,
    },
    Import(ImportDecl),
    ExportFrom(ExportFromDecl),
    ExportList(Vec<ExportSpecifier>),
    ExportDefault(ExportDefaultDecl),
    Stmt(Box<Spanned<Stmt>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportSource {
    Module(String),
    Native(String),
}

#[derive(Clone, Debug)]
pub struct NativeFnSig {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ImportSpecifier {
    pub name: String,
    pub alias: Option<String>,
    pub is_export: bool,
    pub native_fn: Option<NativeFnSig>,
}

#[derive(Clone, Debug)]
pub enum ImportClause {
    Named(Vec<ImportSpecifier>),
    Default(String),
    DefaultAndNamed(String, Vec<ImportSpecifier>),
    Namespace(String),
    Star,
    SideEffect,
}

#[derive(Clone, Debug)]
pub struct ImportDecl {
    pub clause: ImportClause,
    pub source: ImportSource,
    pub source_span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSpecifier {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ExportClause {
    Named(Vec<ExportSpecifier>),
    All { alias: Option<String> },
    Native(Vec<NativeFnSig>),
}

#[derive(Clone, Debug)]
pub struct ExportFromDecl {
    pub clause: ExportClause,
    pub source: ImportSource,
    pub source_span: Span,
}

#[derive(Clone, Debug)]
pub struct ExportDefaultDecl {
    pub expr: Spanned<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RecordField {
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub struct EnumMember {
    pub name: String,
    pub payload: Vec<Type>,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub enum ClassMember {
    Field(FieldDecl),
    Method(FnDecl),
    Init {
        params: Vec<Param>,
        body: Block,
    },
    Deinit(Block),
    OnReload {
        params: Vec<Param>,
        body: Block,
    },
}

#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub access: Access,
    pub mutable: bool,
    pub name: String,
    pub ty: Option<Type>,
    pub value: Option<Spanned<Expr>>,
    pub attrs: Vec<Attr>,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub access: Access,
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub throws: bool,
    pub is_unsafe: bool,
    pub is_async: bool,
    pub is_static: bool,
    pub is_test: bool,
    pub is_bench: bool,
    pub body: FnBody,
    pub attrs: Vec<Attr>,
    pub docs: String,
}

#[derive(Clone, Debug)]
pub enum FnBody {
    Block(Block),
    Expr(Box<Spanned<Expr>>),
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub ty: Option<Type>,
    pub default: Option<Spanned<Expr>>,
    pub promote: bool,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Spanned<Stmt>>,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Var {
        mutable: bool,
        is_static: bool,
        name: String,
        ty: Option<Type>,
        value: Spanned<Expr>,
    },
    DestructureTuple {
        names: Vec<String>,
        ty: Option<Type>,
        value: Spanned<Expr>,
    },
    DestructureRecord {
        fields: Vec<(String, String)>,
        rest: Option<String>,
        value: Spanned<Expr>,
    },
    DestructureArray {
        names: Vec<String>,
        rest: Option<String>,
        value: Spanned<Expr>,
    },
    Assign {
        target: Spanned<Expr>,
        op: AssignOp,
        value: Spanned<Expr>,
    },
    Expr(Spanned<Expr>),
    If {
        cond: IfCond,
        then: Block,
        otherwise: Option<Else>,
    },
    For {
        binding: ForBinding,
        iter: Spanned<Expr>,
        body: Block,
    },
    While {
        cond: Spanned<Expr>,
        body: Block,
    },
    DoWhile {
        body: Block,
        cond: Spanned<Expr>,
    },
    Switch {
        scrutinee: Spanned<Expr>,
        cases: Vec<SwitchCase>,
        default: Option<Vec<Spanned<Stmt>>>,
    },
    Return(Option<Spanned<Expr>>),
    Break,
    Continue,
    Fallthrough,
    Pass,
    Empty,
    Assert(Spanned<Expr>),
    Defer(Block),
    Guard {
        name: String,
        value: Spanned<Expr>,
        otherwise: Block,
    },
    Try {
        body: Block,
        catch: Option<(String, Block)>,
        finally: Option<Block>,
    },
    Throw(Option<Spanned<Expr>>),
    UnsafeBlock(Block),
}

#[derive(Clone, Debug)]
pub enum IfCond {
    Expr(Spanned<Expr>),
    Let { name: String, value: Spanned<Expr> },
}

#[derive(Clone, Debug)]
pub enum Else {
    Block(Block),
    If(Box<Spanned<Stmt>>),
}

#[derive(Clone, Debug)]
pub enum ForBinding {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    pub pattern: Pattern,
    pub guard: Option<Spanned<Expr>>,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Clone, Debug)]
pub struct SwitchExprCase {
    pub pattern: Pattern,
    pub guard: Option<Spanned<Expr>>,
    pub body: SwitchExprBody,
}

#[derive(Clone, Debug)]
pub enum SwitchExprBody {
    Expr(Box<Spanned<Expr>>),
    Block(Block),
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Literal(Spanned<Expr>),
    Range {
        lo: Spanned<Expr>,
        hi: Spanned<Expr>,
        inclusive: bool,
    },
    Is(Type),
    Enum {
        path: Vec<String>,
        args: Vec<Pattern>,
    },
    Wildcard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    ShlEq,
    ShrEq,
    ZshrEq,
}

#[derive(Clone, Debug)]
pub struct ArrayElem {
    pub spread: bool,
    pub expr: Spanned<Expr>,
}

#[derive(Clone, Debug)]
pub enum RecordEntry {
    Field(String, Spanned<Expr>),
    Spread(Spanned<Expr>),
}

#[derive(Clone, Debug)]
pub enum MapEntry {
    Field(String, Spanned<Expr>),
    Spread(Spanned<Expr>),
}

impl RecordEntry {
    pub fn value(&self) -> &Spanned<Expr> {
        match self {
            RecordEntry::Field(_, v) | RecordEntry::Spread(v) => v,
        }
    }

    pub fn map_value(self, f: impl FnOnce(Spanned<Expr>) -> Spanned<Expr>) -> RecordEntry {
        match self {
            RecordEntry::Field(n, v) => RecordEntry::Field(n, f(v)),
            RecordEntry::Spread(v) => RecordEntry::Spread(f(v)),
        }
    }

    pub fn value_mut(&mut self) -> &mut Spanned<Expr> {
        match self {
            RecordEntry::Field(_, v) | RecordEntry::Spread(v) => v,
        }
    }
}

impl MapEntry {
    pub fn value(&self) -> &Spanned<Expr> {
        match self {
            MapEntry::Field(_, v) | MapEntry::Spread(v) => v,
        }
    }

    pub fn map_value(self, f: impl FnOnce(Spanned<Expr>) -> Spanned<Expr>) -> MapEntry {
        match self {
            MapEntry::Field(n, v) => MapEntry::Field(n, f(v)),
            MapEntry::Spread(v) => MapEntry::Spread(f(v)),
        }
    }

    pub fn value_mut(&mut self) -> &mut Spanned<Expr> {
        match self {
            MapEntry::Field(_, v) | MapEntry::Spread(v) => v,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Ident(String),
    This,
    Super,
    Bool(bool),
    Null,
    Int(i64),
    Float(f64),
    Interp(Vec<InterpPart>),
    Array(Vec<ArrayElem>),
    Record(Vec<RecordEntry>),
    MapLiteral(Vec<MapEntry>),
    Binary {
        op: BinOp,
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    Unary {
        op: UnOp,
        rhs: Box<Spanned<Expr>>,
    },
    Postfix {
        op: PostfixOp,
        expr: Box<Spanned<Expr>>,
    },
    Ternary {
        cond: Box<Spanned<Expr>>,
        then: Box<Spanned<Expr>>,
        otherwise: Box<Spanned<Expr>>,
    },
    Coalesce {
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    OptChain {
        base: Box<Spanned<Expr>>,
        field: String,
    },
    OptCall {
        base: Box<Spanned<Expr>>,
        field: String,
        args: Vec<CallArg>,
    },
    Range {
        lo: Box<Spanned<Expr>>,
        hi: Box<Spanned<Expr>>,
        inclusive: bool,
    },
    Call {
        callee: Box<Spanned<Expr>>,
        type_args: Vec<Type>,
        args: Vec<CallArg>,
        trailing: Option<Block>,
    },
    New {
        target: String,
        type_args: Vec<Type>,
        args: Vec<CallArg>,
    },
    Index {
        base: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },
    Member {
        base: Box<Spanned<Expr>>,
        field: String,
    },
    Cast {
        expr: Box<Spanned<Expr>>,
        ty: Type,
    },
    Is {
        base: Box<Spanned<Expr>>,
        target: Type,
    },
    ImplicitMember(Vec<String>),
    Macro {
        name: String,
        args: Vec<Spanned<Expr>>,
        bracket: bool,
    },
    Closure {
        decay: bool,
        is_async: bool,
        params: Vec<Param>,
        ret: Option<Type>,
        throws: bool,
        body: FnBody,
    },
    UnsafeBlock(Block),
    Await(Box<Spanned<Expr>>),
    Propagate(Box<Spanned<Expr>>),
    Tuple(Vec<Spanned<Expr>>),
    TupleGet {
        base: Box<Spanned<Expr>>,
        index: usize,
    },
    Switch {
        scrutinee: Box<Spanned<Expr>>,
        cases: Vec<SwitchExprCase>,
        default: Option<SwitchExprBody>,
    },
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Text(String),
    Expr(Spanned<Expr>),
}

#[derive(Clone, Debug)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Spanned<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Zshr,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    AddrOf,
    Deref,
    PreInc,
    PreDec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostfixOp {
    PostInc,
    PostDec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub path: Vec<String>,
    pub args: Vec<Type>,
    pub nullable: bool,
    pub fn_sig: Option<FnType>,
    pub tuple: Vec<Type>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnType {
    pub params: Vec<Type>,
    pub ret: Option<Box<Type>>,
}

impl Type {
    pub fn named(name: &str) -> Type {
        Type {
            path: vec![name.to_string()],
            args: Vec::new(),
            nullable: false,
            fn_sig: None,
            tuple: Vec::new(),
        }
    }
}
