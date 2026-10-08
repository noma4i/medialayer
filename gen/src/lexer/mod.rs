use crate::ParseError;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Word(String),
    Symbol(String),
    Int(u64),
    Quantity { amount: u64, unit: String },
    Range,
    LeftBracket,
    RightBracket,
    Comma,
    Colon,
    Newline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Kind,
    pub line: usize,
    pub column: usize,
}

pub const UNITS: [&str; 11] =
    ["second", "seconds", "minute", "minutes", "hour", "hours", "day", "days", "mib", "gib", "percent"];

struct Cursor {
    chars: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
}

impl Cursor {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let character = self.peek(0)?;
        self.index += 1;
        if character == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(character)
    }

    fn take_while(&mut self, accepts: impl Fn(char) -> bool) -> String {
        let mut text = String::new();
        while let Some(character) = self.peek(0).filter(|character| accepts(*character)) {
            text.push(character);
            self.advance();
        }
        text
    }
}

fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

pub fn lex(source: &str) -> Result<Vec<Token>, ParseError> {
    let mut cursor = Cursor { chars: source.chars().collect(), index: 0, line: 1, column: 1 };
    let mut tokens: Vec<Token> = Vec::new();
    while let Some(character) = cursor.peek(0) {
        let (line, column) = (cursor.line, cursor.column);
        let kind = match character {
            ' ' | '\t' | '\r' => {
                cursor.advance();
                continue;
            }
            '#' => {
                cursor.take_while(|character| character != '\n');
                continue;
            }
            '\n' => {
                cursor.advance();
                if matches!(tokens.last(), Some(Token { kind: Kind::Comma, .. })) {
                    continue;
                }
                Kind::Newline
            }
            '[' | ']' | ',' => {
                cursor.advance();
                match character {
                    '[' => Kind::LeftBracket,
                    ']' => Kind::RightBracket,
                    _ => Kind::Comma,
                }
            }
            '.' if cursor.peek(1) == Some('.') && cursor.peek(2) == Some('.') => {
                (0..3).for_each(|_| {
                    cursor.advance();
                });
                Kind::Range
            }
            ':' if cursor.peek(1).is_some_and(is_name_character) => {
                cursor.advance();
                Kind::Symbol(cursor.take_while(is_name_character))
            }
            ':' => {
                cursor.advance();
                Kind::Colon
            }
            digit if digit.is_ascii_digit() => lex_number(&mut cursor, line, column)?,
            letter if letter.is_alphabetic() || letter == '_' => Kind::Word(cursor.take_while(is_name_character)),
            other => return Err(ParseError::new(line, column, format!("unexpected character `{other}`"))),
        };
        tokens.push(Token { kind, line, column });
    }
    Ok(tokens)
}

fn lex_number(cursor: &mut Cursor, line: usize, column: usize) -> Result<Kind, ParseError> {
    let digits = cursor.take_while(|character| character.is_ascii_digit());
    let amount: u64 = digits.parse().map_err(|_| ParseError::new(line, column, "number is too large"))?;
    if cursor.peek(0) == Some('.') && cursor.peek(1).is_some_and(char::is_alphabetic) {
        cursor.advance();
        let unit = cursor.take_while(is_name_character);
        if !UNITS.contains(&unit.as_str()) {
            return Err(ParseError::new(line, column, format!("unknown unit `{unit}`")));
        }
        return Ok(Kind::Quantity { amount, unit });
    }
    Ok(Kind::Int(amount))
}

#[cfg(test)]
mod lex_tests;
