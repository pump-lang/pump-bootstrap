// chuong trinh dich duoc roi chet co y.
//
// panic la mot phan hanh vi nhin thay duoc cua ngon ngu - ca spec lan grammar
// deu ke ten nhung phep toan co panic - nen moi ca ghim ma thoat, ghim phan
// stdout in ra truoc khi chet, va ghim ca dong thong bao.

mod support;

macro_rules! panic_cases {
    ($($name:ident,)*) => {
        $(
            #[test]
            fn $name() {
                support::check_panic_case(stringify!($name));
            }
        )*

        #[test]
        fn every_case_is_registered() {
            support::check_registry("panic", "panic", &[$(stringify!($name)),*]);
        }
    };
}

panic_cases! {
    array_index_out_of_range,
    missing_map_key,
    pop_from_empty_array,
    divide_by_zero,
    mutate_while_iterating,
    expect_on_null,
    explicit_panic,
    failed_assert,
}
