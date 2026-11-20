use crate::*;

// 编译即验证：不构造容器，不能触发构造、业务或 Drop 桩。
#[test]
fn thread_trait_bounds() {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<AppendVec<std::cell::Cell<u8>>>();
    send::<AppendVec<String>>();
    sync::<AppendVec<String>>();
    let _: Option<AppendVec<std::rc::Rc<u8>>> = None;
}

// 原有验收场景保留：当前只编译，设计桩完成前不运行。
#[test]
fn test() {
    println!("test start");
    let mut vec = AppendVec::with_capacity(4);
    let _good_day = vec.push("Good day");
    let _hello = vec.push("Hello");
    assert_eq!(vec.len(), 2);
    let hello1 = vec.push("Hello");
    assert_eq!(vec[hello1], "Hello");
    assert_eq!(unsafe { vec.get_unchecked(hello1) }, &"Hello");
    println!("test: {:?}", 2);
    *vec.get_mut(hello1).unwrap() = "Hello1";
    assert_eq!(vec[hello1], "Hello1");
    assert_eq!(vec.len(), 3);
    println!("test: {:?}", 3);
    println!("vec: {:?}", vec);
}
#[test]
fn test_removes() {
    let mut removes: AppendVec<usize> = Default::default();
    removes.push(1);
    removes.push(2);
    removes.clear(1);
    removes.push(1);
    removes.push(6);
}

#[test]
fn test_str() {
    let vec = AppendVec::with_capacity(4);
    let _good_day = vec.push("Good day");
    let _hello = vec.push("Hello");
    assert_eq!(vec.len(), 2);
    let hello1 = vec.push("Hello");
    assert_eq!(vec[hello1], "Hello");
}
