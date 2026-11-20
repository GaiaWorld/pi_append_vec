//! 追加与按序发布容器；访问约束见各容器壳模块。
pub mod append_vec;
pub use append_vec::AppendVec;

#[cfg(test)]
mod tests;
