use super::*;

#[gpui::test]
async fn folder_actions_for_listed_members_leave_the_other_members_alone(
    cx: &mut gpui::TestAppContext,
) {
    let grouped = |name: &str| {
        let mut item = profile(name);
        item.group = Some("Work".into());
        item
    };
    let (mine, theirs) = (grouped("mine"), grouped("theirs"));
    let entity = cx.new(|_| Connections::in_memory());
    for item in [&mine, &theirs] {
        entity
            .update(cx, |e, cx| e.save_profile(item.clone(), cx))
            .await
            .unwrap();
    }
    let group_of = |id, cx: &mut gpui::TestAppContext| {
        entity.read_with(cx, |e, cx| e.profiles(cx).get(id).unwrap().group.clone())
    };
    entity
        .update(cx, |e, cx| {
            e.rename_group_of(vec![mine.id], "Work".into(), " Team ".into(), cx)
        })
        .await
        .unwrap();
    assert_eq!(group_of(mine.id, cx), Some("Team".into()));
    assert_eq!(group_of(theirs.id, cx), Some("Work".into()));

    entity
        .update(cx, |e, cx| e.ungroup_of(vec![mine.id], "Team".into(), cx))
        .await
        .unwrap();
    assert_eq!(group_of(mine.id, cx), None);
    assert_eq!(group_of(theirs.id, cx), Some("Work".into()));

    entity
        .update(cx, |e, cx| {
            e.save_profile(
                Profile {
                    group: Some("Work".into()),
                    ..mine.clone()
                },
                cx,
            )
        })
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| {
            e.delete_group_of("Work".into(), vec![mine.id], cx)
        })
        .await
        .unwrap();
    entity.read_with(cx, |e, cx| {
        assert!(e.profiles(cx).get(mine.id).is_none());
        assert!(e.profiles(cx).get(theirs.id).is_some());
    });
}

#[gpui::test]
async fn deleting_listed_members_is_refused_when_one_left_the_folder(
    cx: &mut gpui::TestAppContext,
) {
    let mut a = profile("a");
    a.group = Some("Work".into());
    let entity = cx.new(|_| Connections::in_memory());
    entity
        .update(cx, |e, cx| e.save_profile(a.clone(), cx))
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| e.move_profile(a.id, None, cx))
        .await
        .unwrap();
    entity
        .update(cx, |e, cx| e.delete_group_of("Work".into(), vec![a.id], cx))
        .await
        .unwrap_err();
    assert!(entity.read_with(cx, |e, cx| e.profiles(cx).get(a.id).is_some()));
}
