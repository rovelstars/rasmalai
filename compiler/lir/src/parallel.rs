pub(crate) fn with_pool<T>(jobs: usize, op: impl FnOnce() -> T + Send) -> T
where
    T: Send,
{
    match rayon::ThreadPoolBuilder::new().num_threads(jobs).build() {
        Ok(pool) => pool.install(op),
        Err(_) => op(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAR_SRC: &str = "class Box {\n\
        let val: Int;\n\
        init(v: Int) { this.val = v; }\n\
        get(): Int { return this.val; }\n\
        add(x: Int): Int { return this.val + x; }\n\
        }\n\
        fn fib(n: Int): Int {\n\
        if n < 2 {\n\
        return n;\n\
        }\n\
        return fib(n - 1) + fib(n - 2);\n\
        }\n\
        fn sumTo(n: Int): Int {\n\
        let total = 0;\n\
        let i = 0;\n\
        while i <= n {\n\
        total = total + i;\n\
        i = i + 1;\n\
        }\n\
        return total;\n\
        }\n\
        fn withClosures(base: Int): Int {\n\
        let factor = 3;\n\
        let mult = (x: Int): Int => x * factor;\n\
        let addBase = (x: Int): Int => x + base;\n\
        let mk = (a: Int) => (b: Int): Int => a + b;\n\
        let add5 = mk(5);\n\
        let nested = (x: Int): Int => mult(x) + addBase(x) + add5(x);\n\
        return nested(10);\n\
        }\n\
        fn arrSum(): Int {\n\
        let arr = [10, 20, 30, 40];\n\
        let sum = 0;\n\
        let i = 0;\n\
        while i < arr.length {\n\
        sum = sum + arr[i];\n\
        i = i + 1;\n\
        }\n\
        return sum;\n\
        }\n\
        fn useBox(n: Int): Int {\n\
        let b = new Box(n);\n\
        return b.add(fib(10));\n\
        }\n\
        fn Main(): Int {\n\
        let a = fib(15);\n\
        let b = sumTo(100);\n\
        let c = withClosures(7);\n\
        let d = useBox(5);\n\
        return a + b + c + d + arrSum();\n\
        }\n";

    fn lowered(src: &str) -> frontend::ast::Module {
        let mut m =
            frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("parse: {e}"));
        let d = frontend::desugar::desugar(&mut m);
        assert!(d.is_empty(), "{d:?}");
        m
    }

    #[test]
    fn parallel_lower_matches_sequential() {
        let m = lowered(PAR_SRC);
        let expect = crate::lower::lower(&m).unwrap_or_else(|e| panic!("lower: {e}"));
        let want = format!("{:?}", expect);
        for jobs in [0, 1, 2, 4, 8] {
            let got = crate::lower::lower_parallel(&m, jobs)
                .unwrap_or_else(|e| panic!("parallel lower jobs={jobs}: {e}"));
            assert_eq!(format!("{:?}", got), want, "jobs={jobs}");
        }
    }

    #[test]
    fn parallel_opt_matches_sequential() {
        let m = lowered(PAR_SRC);
        let base = crate::lower::lower(&m).unwrap_or_else(|e| panic!("lower: {e}"));
        let mut expect = base.clone();
        crate::opt::optimize_lir(&mut expect, 1, "Main");
        let want = format!("{:?}", expect);
        for jobs in [0, 1, 2, 4, 8] {
            let mut got = base.clone();
            crate::opt::optimize_lir_parallel(&mut got, 1, "Main", jobs);
            assert_eq!(format!("{:?}", got), want, "jobs={jobs}");
        }
        let mut expect_lib = base.clone();
        crate::opt::optimize_lir_lib(&mut expect_lib, 1);
        let want_lib = format!("{:?}", expect_lib);
        for jobs in [0, 1, 2, 4, 8] {
            let mut got = base.clone();
            crate::opt::optimize_lir_lib_parallel(&mut got, 1, jobs);
            assert_eq!(format!("{:?}", got), want_lib, "lib jobs={jobs}");
        }
    }

    #[test]
    fn parallel_verify_matches_sequential() {
        let m = lowered(PAR_SRC);
        let base = crate::lower::lower(&m).unwrap_or_else(|e| panic!("lower: {e}"));
        let mut opt = base.clone();
        crate::opt::optimize_lir(&mut opt, 1, "Main");
        for module in [&base, &opt] {
            let want = crate::verify::verify(module);
            for jobs in [0, 1, 2, 4, 8] {
                let got = crate::verify::verify_parallel(module, jobs);
                assert_eq!(format!("{:?}", got), format!("{:?}", want), "jobs={jobs}");
            }
        }
    }

    fn big_source(n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            let next = if i + 1 < n {
                format!(" + f{}(x)", i + 1)
            } else {
                String::new()
            };
            s.push_str(&format!(
                "fn f{i}(x: Int): Int {{\nlet t = x * 3 + {i};\nlet j = 0;\nlet s = 0;\nwhile j < 50 {{\ns = s + t + j;\nj = j + 1;\n}}\nreturn s{next};\n}}\n"
            ));
        }
        s.push_str("fn Main(): Int {\nreturn f0(1);\n}\n");
        s
    }

    #[test]
    fn timing_parallel_vs_sequential() {
        use std::time::Instant;
        let src = big_source(300);
        let m = lowered(&src);
        let t = Instant::now();
        let seq = crate::lower::lower(&m).unwrap_or_else(|e| panic!("lower: {e}"));
        let lower_seq = t.elapsed();
        let t = Instant::now();
        let par = crate::lower::lower_parallel(&m, 8)
            .unwrap_or_else(|e| panic!("parallel lower: {e}"));
        let lower_par = t.elapsed();
        assert_eq!(format!("{:?}", par), format!("{:?}", seq));
        let t = Instant::now();
        let mut a = seq.clone();
        crate::opt::optimize_lir(&mut a, 1, "Main");
        let opt_seq = t.elapsed();
        let t = Instant::now();
        let mut b = seq.clone();
        crate::opt::optimize_lir_parallel(&mut b, 1, "Main", 8);
        let opt_par = t.elapsed();
        assert_eq!(format!("{:?}", b), format!("{:?}", a));
        eprintln!(
            "lower seq={}ms par8={}ms opt seq={}ms par8={}ms fns={}",
            lower_seq.as_millis(),
            lower_par.as_millis(),
            opt_seq.as_millis(),
            opt_par.as_millis(),
            seq.functions.len()
        );
    }
}
