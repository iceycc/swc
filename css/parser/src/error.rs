use std::borrow::Cow;

use swc_common::{
    errors::{DiagnosticBuilder, Handler},
    Span,
};

/// Size is same as a size of a pointer.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    inner: Box<(Span, ErrorKind)>,
}

impl Error {
    pub fn kind(&self) -> &ErrorKind {
        &self.inner.1
    }

    pub fn into_inner(self) -> Box<(Span, ErrorKind)> {
        self.inner
    }

    pub fn new(span: Span, kind: ErrorKind) -> Self {
        Error {
            inner: Box::new((span, kind)),
        }
    }

    pub fn to_diagnostics<'a>(&self, handler: &'a Handler) -> DiagnosticBuilder<'a> {
        let msg: Cow<_> = match self.inner.1 {
            ErrorKind::Eof => "Unexpected end of file".into(),
            ErrorKind::UnexpectedChar(c) => format!("Unexpected charcter `{:?}`", c).into(),
            ErrorKind::UnterminatedUrl => "Unterminated url literal".into(),
            ErrorKind::InvalidEscape => "Invalid escape".into(),
            ErrorKind::Expected(s) => format!("Expected {}", s).into(),
            ErrorKind::ExpectedButGot(s) => format!("Expected {}", s).into(),
            ErrorKind::ExpectedSelectorText => "Expected a text for selector".into(),
            ErrorKind::UnterminatedBlockComment => "Unterminated block commment".into(),
            ErrorKind::InvalidTypeSelector => "Invalid type selector".into(),
            ErrorKind::InvalidSelector => "Invalid selector".into(),
            ErrorKind::ExpectedIdentOrStrForAttrSelectorOp => {
                "Expected an identifier or a string after an attribute selector operator".into()
            }
            ErrorKind::ExpectedNumber => "Expected a number".into(),
            ErrorKind::InvalidSupportQuery => "Invalid support query".into(),
            ErrorKind::InvalidMediaQuery => "Invalid media query".into(),
            ErrorKind::UnknownAtRuleNotTerminated => "Unknown @rule is not terminated".into(),
        };
        handler.struct_span_err(self.inner.0, &msg)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    Eof,
    /// Lexing error.
    UnexpectedChar(Option<char>),
    /// Lexing error.
    UnterminatedUrl,
    /// Lexing error
    InvalidEscape,
    Expected(&'static str),
    ExpectedButGot(&'static str),
    ExpectedSelectorText,
    UnterminatedBlockComment,
    InvalidTypeSelector,
    InvalidSelector,
    ExpectedIdentOrStrForAttrSelectorOp,
    ExpectedNumber,
    InvalidSupportQuery,
    InvalidMediaQuery,

    UnknownAtRuleNotTerminated,
}
