use super::*;

#[test]
fn requests_preserve_literal_arguments_and_native_cwd_without_interpolation() {
    let args = vec![
        "a b".into(),
        "'quoted'".into(),
        "$(touch marker);$HOME".into(),
        "".into(),
    ];
    let cwd = PathBuf::from("folder with spaces");
    let request =
        ProgramOpenRequest::new(" editor ".into(), args.clone(), Some(cwd.clone())).unwrap();
    assert_eq!(request.program(), " editor ");
    assert_eq!(request.args(), args);
    assert_eq!(request.cwd(), Some(cwd.as_path()));
    assert_eq!(request.into_parts(), (" editor ".into(), args, Some(cwd)));
}

#[test]
fn requests_reject_only_empty_executables_and_native_nul_arguments() {
    for program in ["", "bad\0program"] {
        assert!(matches!(
            ProgramOpenRequest::new(program.into(), vec![], None),
            Err(ProgramOpenError::InvalidProgram)
        ));
    }
    assert!(matches!(
        ProgramOpenRequest::new(
            "editor".into(),
            vec!["good".into(), "bad\0arg".into()],
            None
        ),
        Err(ProgramOpenError::InvalidArgument { index: 1 })
    ));
    ProgramOpenRequest::new("editor".into(), vec!["x".repeat(1024 * 1024)], None).unwrap();
}
