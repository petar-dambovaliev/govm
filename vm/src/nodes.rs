use parser::ast::{Declaration, Expression, Statement};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

enum Word {
    // Special words
    Illegal,

    // Names and basic type literals
    // (these words stand for classes of literals)
    Name,   // main
    Int,    // 12345
    Float,  // 123.45
    Imag,   // 123.45i
    Char,   // 'a'
    String, // "abc"

    // Operators and delimiters
    Add, // +
    Sub, // -
    Mul, // *
    Quo, // /
    Rem, // %

    Band,    // &
    Bor,     // |
    Xor,     // ^
    Shl,     // <<
    Shr,     // >>
    BandNot, // &^

    AddAssign,     // +=
    SubAssign,     // -=
    MulAssign,     // *=
    QuoAssign,     // /=
    RemAssign,     // %=
    BandAssign,    // &=
    BorAssign,     // |=
    XorAssign,     // ^=
    ShlAssign,     // <<=
    ShrAssign,     // >>=
    BandNotAssign, // &^=

    Land,  // &&
    Lor,   // ||
    Arrow, // <-
    Inc,   // ++
    Dec,   // --

    Eql,    // ==
    Lss,    // <
    Gtr,    // >
    Assign, // =
    Not,    // !

    Neq,    // !=
    Leq,    // <=
    Geq,    // >=
    Define, // :=

    // Keywords
    Break,
    Case,
    Chan,
    Const,
    Continue,

    Default,
    Defer,
    Else,
    Fallthrough,
    For,

    Func,
    GO,
    Goto,
    IF,
    Import,

    Interface,
    Map,
    Package,
    Range,
    Return,

    Select,
    Struct,
    Switch,
    Type,
    Var,
}

type Name = String;

// ----------------------------------------
// Location
// Acts as an identifier for nodes.
struct Location {
    pub pkg_path: String,
    pub file: String,
    pub line: isize,
    pub nonce: isize,
}

impl Location {
    fn is_zero(&self) -> bool {
        self.pkg_path.is_empty() && self.file.is_empty() && self.line == 0 && self.nonce == 0
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.nonce == 0 {
            return write!(f, "{}/{}:{}", self.pkg_path, self.file, self.line);
        }
        write!(
            f,
            "{}/{}:{}#{}",
            self.pkg_path, self.file, self.line, self.nonce
        )
    }
}

// ----------------------------------------
// Attributes
// All nodes have attributes for general analysis purposes.
// Exported Attribute fields like Loc and Label are persisted
// even after preprocessing.  Temporary attributes (e.g. those
// for preprocessing) are stored in .data.

pub enum AttrKey {
    Preprocessed,
    Predefined,
    TypeValue,
    TypeOfValue,
    Iota,
    Locationed,
    Injected,
}

pub enum AttrVal {
    TypedValue,
    Isize,
    Bool,
}

impl AttrKey {
    pub fn as_str(&self) -> &'static str {
        match &self {
            Self::Preprocessed => "ATTR_PREPROCESSED",
            Self::Predefined => "ATTR_PREDEFINED",
            Self::TypeValue => "ATTR_TYPE_VALUE",
            Self::TypeOfValue => "ATTR_TYPEOF_VALUE",
            Self::Iota => "ATTR_IOTA",
            Self::Locationed => "ATTR_LOCATIONED",
            Self::Injected => "ATTR_INJECTED",
        }
    }
}

struct Attributes {
    pub line: isize,
    pub label: Name,
    data: HashMap<AttrKey, AttrVal>,
}

#[derive(Debug)]
pub enum Node {
    Statement(Statement),
    Expr(Expression),
    Decl(Declaration),
}
//NameExpr,
// BasicLitExpr(BasicLit),
// BinaryExpr(Operation),
// CallExpr(Call),
// IndexExpr(Index),
// SelectorExpr(Selector),
// SliceExpr(Slice),
// StarExpr(StarExpression),
// RefExpr(Ref),
// TypeAssertExpr,
// UnaryExpr,
// CompositeLitExpr,
// KeyValueExpr,
// FuncLitExpr,
// ConstExpr,
// FieldTypeExpr,
// ArrayTypeExpr,
// SliceTypeExpr,
// InterfaceTypeExpr,
// ChanTypeExpr,
// FuncTypeExpr,
// MapTypeExpr,
// StructTypeExpr,
// ConstTypeExpr,
// MaybeNativeTypeExpr,
// AssignStmt,
// BlockStmt,
// BranchStmt,
// DeclStmt,
// DeferStmt,
// ExprStmt,
// ForStmt,
// GoStmt,
// IfStmt,
// IfCaseStmt,
// IncDecStmt,
// RangeStmt,
// ReturnStmt,
// PanicStmt,
// SelectStmt,
// SelectCaseStmt,
// SendStmt,
// SwitchStmt,
// SwitchClauseStmt,
// EmptyStmt,
// BodyStmt,
// FuncDecl,
// ImportDecl,
// ValueDecl,
// TypeDecl,
// FileNode,
// PackageNode,

// assertNode()
// String() string
// Copy() Node
// GetLine() int
// SetLine(int)
// GetLabel() Name
// SetLabel(Name)
// HasAttribute(key interface{}) bool
// GetAttribute(key interface{}) interface{}
// SetAttribute(key interface{}, value interface{})
