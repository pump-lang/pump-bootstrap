// chuong trinh phai dich duoc va chay duoc, so voi dung stdout cua no.
//
// Moi ca la tests/cases/run/NAME.pump nam canh tests/cases/run/NAME.out.
// Danh sach o duoi la so dang ky: every_case_is_registered se do neu co
// chuong trinh nam tren dia ma thieu trong danh sach, hoac nguoc lai.

mod support;

macro_rules! run_cases {
    ($($name:ident,)*) => {
        $(
            #[test]
            fn $name() {
                support::check_run_case(stringify!($name));
            }
        )*

        #[test]
        fn every_case_is_registered() {
            support::check_registry("run", "out", &[$(stringify!($name)),*]);
        }
    };
}

run_cases! {
    // muc 1: hinh dang mot chuong trinh.
    hello_world,
    statement_termination,
    prelude,

    // muc 2 va 21: buoc ten va hang.
    bindings,
    compound_assignment,

    // muc 3: kieu co ban, literal, noi suy.
    primitive_types,
    numeric_literals,
    char_and_string,
    string_interpolation,
    conversions,
    integer_semantics,

    // muc 4: collection.
    arrays,
    maps,
    sets,
    tuples,

    // muc 5 va 6: ham va tham so.
    functions,
    parameters,

    // muc 7 den 10: re nhanh va vong lap.
    if_else,
    while_loop,
    for_ranges,
    for_collections,
    ranges_as_values,
    break_continue,

    // muc 11 va 12: struct va method.
    structs,
    methods,
    method_self_call,

    // muc 13 va 14: enum va match.
    enums,
    match_statement,
    match_patterns,

    // muc 15: optional.
    optionals,
    optional_propagation,
    match_optional,

    // muc 16: xu ly loi.
    errors,
    catch_forms,
    catch_edge_cases,
    optional_and_error_types,

    // muc 17 va 18: generic va interface.
    generics,
    interfaces,

    // muc 20 den 25: visibility, closure, toan tu, comment.
    visibility_same_module,
    closures,
    operators,
    comments,

    // cai stdlib be di kem.
    string_library,

    // nhieu thu mot luc.
    combined_inventory,
    combined_expression_tree,
    combined_generic_pipeline,
    combined_gc_pressure,
}
