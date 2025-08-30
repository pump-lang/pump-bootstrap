// chuong trinh nhieu file: import, duong dan module long nhau, dat ten khac,
// va visibility qua mot ranh gioi module that.

mod support;

macro_rules! project_cases {
    ($($name:ident,)*) => {
        $(
            #[test]
            fn $name() {
                support::check_project_case(stringify!($name));
            }
        )*

        #[test]
        fn every_case_is_registered() {
            support::check_project_registry(&[$(stringify!($name)),*]);
        }
    };
}

project_cases! {
    // muc 19: import thuong, import long, import dat ten khac.
    modules,
    // mot interface, may ban cai dat, va mot cho tra cuu co the that bai, moi thu mot module.
    layered,
}
