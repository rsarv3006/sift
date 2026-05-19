/// Arithmetic operation variants: add, subtract, multiply, divide, and pow (exponentiation).
pub enum Op {
    /// Addition
    Add,
    /// Subtraction
    Subtract,
    /// Multiplication
    Multiply,
    /// Division
    Divide,
    /// Exponentiation / power
    Pow,
}

/// Trait for arithmetic operations that can be applied to two f64 values.
pub trait Operation {
    /// Human-readable operation name.
    fn name(&self) -> &'static str;
    /// Apply the operation to two operands and return the result.
    fn apply(&self, a: f64, b: f64) -> f64;
}

pub struct AddOp;
pub struct MulOp;
pub struct DivOp;

impl Operation for AddOp {
    fn name(&self) -> &'static str { "add" }
    fn apply(&self, a: f64, b: f64) -> f64 { a + b }
}

impl Operation for MulOp {
    fn name(&self) -> &'static str { "mul" }
    fn apply(&self, a: f64, b: f64) -> f64 { a * b }
}

impl Operation for DivOp {
    fn name(&self) -> &'static str { "div" }
    fn apply(&self, a: f64, b: f64) -> f64 { a / b }
}

/// Add two numbers together.
pub fn add(a: f64, b: f64) -> f64 { a + b }
/// Subtract b from a.
pub fn subtract(a: f64, b: f64) -> f64 { a - b }
/// Multiply a by b.
pub fn multiply(a: f64, b: f64) -> f64 { a * b }
/// Divide a by b. Panics on division by zero.
pub fn divide(a: f64, b: f64) -> f64 {
    if b == 0.0 { panic!("division by zero") }
    a / b
}
