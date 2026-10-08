use crate::lexer::{Kind, Token};
use crate::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Symbol(String),
    Int(u64),
    Quantity { amount: u64, unit: String },
    Range { from: u64, to: u64 },
    Array(Vec<Node<Value>>),
    Call { word: String, arg: Box<Node<Value>> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node<T> {
    pub item: T,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pair {
    pub key: String,
    pub line: usize,
    pub column: usize,
    pub value: Node<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub word: String,
    pub line: usize,
    pub column: usize,
    pub args: Vec<Node<Value>>,
    pub options: Vec<Pair>,
    pub block: Option<Vec<Call>>,
}

struct Reader<'a> {
    tokens: &'a [Token],
    index: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.index)
    }

    fn peek_kind(&self, offset: usize) -> Option<&'a Kind> {
        self.tokens.get(self.index + offset).map(|token| &token.kind)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.peek();
        self.index += 1;
        token
    }

    fn at_statement_end(&self) -> bool {
        matches!(self.peek_kind(0), None | Some(Kind::Newline))
    }

    fn at_word(&self, expected: &str) -> bool {
        matches!(self.peek_kind(0), Some(Kind::Word(word)) if word == expected)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(0), Some(Kind::Newline)) {
            self.index += 1;
        }
    }

    fn unexpected(&self) -> ParseError {
        match self.peek() {
            Some(token) => ParseError::new(token.line, token.column, format!("unexpected {}", describe(&token.kind))),
            None => {
                let (line, column) = self.tokens.last().map_or((1, 1), |token| (token.line, token.column));
                ParseError::new(line, column, "unexpected end of file")
            }
        }
    }
}

fn describe(kind: &Kind) -> String {
    match kind {
        Kind::Word(text) => format!("`{text}`"),
        Kind::Symbol(text) => format!("`:{text}`"),
        Kind::Int(number) => format!("`{number}`"),
        Kind::Quantity { amount, unit } => format!("`{amount}.{unit}`"),
        Kind::Range => "`...`".to_string(),
        Kind::LeftBracket => "`[`".to_string(),
        Kind::RightBracket => "`]`".to_string(),
        Kind::Comma => "`,`".to_string(),
        Kind::Colon => "`:`".to_string(),
        Kind::Newline => "end of line".to_string(),
    }
}

pub fn parse(tokens: &[Token]) -> Result<Vec<Call>, ParseError> {
    let mut reader = Reader { tokens, index: 0 };
    let mut calls = Vec::new();
    loop {
        reader.skip_newlines();
        if reader.peek().is_none() {
            return Ok(calls);
        }
        calls.push(parse_call(&mut reader)?);
    }
}

fn parse_call(reader: &mut Reader) -> Result<Call, ParseError> {
    let Some(Token { kind: Kind::Word(word), line, column }) = reader.next() else {
        reader.index -= 1;
        return Err(reader.unexpected());
    };
    let mut call = Call { word: word.clone(), line: *line, column: *column, args: Vec::new(), options: Vec::new(), block: None };
    while !reader.at_statement_end() && !reader.at_word("do") {
        parse_item(reader, &mut call)?;
        if !matches!(reader.peek_kind(0), Some(Kind::Comma)) {
            break;
        }
        reader.next();
        if reader.at_statement_end() {
            return Err(reader.unexpected());
        }
    }
    if reader.at_word("do") {
        reader.next();
        call.block = Some(parse_block(reader, &call)?);
    }
    if !reader.at_statement_end() {
        return Err(reader.unexpected());
    }
    Ok(call)
}

fn parse_block(reader: &mut Reader, opener: &Call) -> Result<Vec<Call>, ParseError> {
    if !reader.at_statement_end() {
        return Err(reader.unexpected());
    }
    let mut calls = Vec::new();
    loop {
        reader.skip_newlines();
        if reader.peek().is_none() {
            let message = format!("`{}` block is not closed with `end`", opener.word);
            return Err(ParseError::new(opener.line, opener.column, message));
        }
        if reader.at_word("end") {
            reader.next();
            return Ok(calls);
        }
        calls.push(parse_call(reader)?);
    }
}

fn parse_item(reader: &mut Reader, call: &mut Call) -> Result<(), ParseError> {
    if let (Some(Kind::Word(_)), Some(Kind::Colon)) = (reader.peek_kind(0), reader.peek_kind(1)) {
        let Some(Token { kind: Kind::Word(key), line, column }) = reader.next() else { unreachable!("word checked above") };
        reader.next();
        call.options.push(Pair { key: key.clone(), line: *line, column: *column, value: parse_value(reader)? });
    } else {
        call.args.push(parse_value(reader)?);
    }
    Ok(())
}

fn parse_value(reader: &mut Reader) -> Result<Node<Value>, ParseError> {
    let Some(token) = reader.peek() else {
        return Err(reader.unexpected());
    };
    let item = match &token.kind {
        Kind::Symbol(text) => Value::Symbol(text.clone()),
        Kind::Int(from) if matches!(reader.peek_kind(1), Some(Kind::Range)) => {
            reader.next();
            reader.next();
            let Some(Token { kind: Kind::Int(to), .. }) = reader.peek() else {
                return Err(reader.unexpected());
            };
            Value::Range { from: *from, to: *to }
        }
        Kind::Int(number) => Value::Int(*number),
        Kind::Quantity { amount, unit } => Value::Quantity { amount: *amount, unit: unit.clone() },
        Kind::LeftBracket => return parse_array(reader),
        Kind::Word(word) if word == "expire_after" => {
            reader.next();
            let arg = parse_value(reader)?;
            return Ok(Node { item: Value::Call { word: word.clone(), arg: Box::new(arg) }, line: token.line, column: token.column });
        }
        _ => return Err(reader.unexpected()),
    };
    reader.next();
    Ok(Node { item, line: token.line, column: token.column })
}

fn parse_array(reader: &mut Reader) -> Result<Node<Value>, ParseError> {
    let open = reader.next().expect("bracket checked by caller");
    let mut items = Vec::new();
    loop {
        reader.skip_newlines();
        if matches!(reader.peek_kind(0), Some(Kind::RightBracket)) {
            reader.next();
            return Ok(Node { item: Value::Array(items), line: open.line, column: open.column });
        }
        items.push(parse_value(reader)?);
        reader.skip_newlines();
        match reader.peek_kind(0) {
            Some(Kind::Comma) => {
                reader.next();
            }
            Some(Kind::RightBracket) => {}
            _ => return Err(reader.unexpected()),
        }
    }
}

#[cfg(test)]
mod parse_tests;
