use frontend::check::check_source;

#[test]
fn instance_method_collection_block_passes_semantics() {
    let src = r#"
class Box {
    fn make(children: Array<Int>): Int { return children.length() }
    fn run(): Int { return this.make { 1 2 } }
}
fn Main(): Int {
    let b = new Box()
    let n = b.make { 1 2 3 }
    return n + b.run()
}
"#;
    assert!(check_source(src).is_ok(), "{:?}", check_source(src));
}

#[test]
fn free_function_collection_block_passes_semantics() {
    let src = r#"
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { return F { 1 2 3 } }
"#;
    assert!(check_source(src).is_ok(), "{:?}", check_source(src));
}
