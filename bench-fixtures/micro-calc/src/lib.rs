mod ops;
mod test_utils;

use ops::{add, subtract, multiply, divide, AddOp, MulOp, DivOp, Operation, Op};

/// A calculator that chains operations and evaluates them sequentially.
pub struct Calculator {
    pub ops: Vec<Op>,
}

impl Calculator {
    /// Create a new calculator with an empty operation chain.
    pub fn new() -> Self {
        Calculator { ops: Vec::new() }
    }

    /// Append an operation to the chain.
    pub fn add(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Evaluate the chained operation list starting from `a`, using `b` as the second operand.
    /// Handles addition, subtraction, multiplication, division, and exponentiation.
    pub fn evaluate(&self, a: f64, b: f64) -> f64 {
        let mut result = a;
        for op in &self.ops {
            result = match op {
                Op::Add => add(result, b),
                Op::Subtract => subtract(result, b),
                Op::Multiply => multiply(result, b),
                Op::Divide => divide(result, b),
                Op::Pow => result.powf(b),
            };
        }
        result
    }
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

/// Run an operation trait object with two operands.
pub fn run_operation(op: &dyn Operation, a: f64, b: f64) -> f64 {
    op.apply(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculator_add() {
        let mut calc = Calculator::new();
        calc.add(Op::Add);
        assert_eq!(calc.evaluate(3.0, 4.0), 7.0);
    }
}
