import io
path = r"D:\github-project\liquid-verre-rs\crates\liquid-verre\src\api\handle.rs"
with io.open(path, "r", encoding="utf-8") as f:
    c = f.read()

anchor = "    #[test]\n    fn events_are_queued_and_drained_once() {"
assert anchor in c, "anchor not found"

new_tests = """    #[test]
    fn update_content_valid_input_enqueues_upload_command() {
        let (shared, w) = pair();
        // 2x1 BGRA8 = 8 字节，合法；dirty 半开区间原样带进命令（裁剪在 gfx 消费时做）。
        w.update_content(2, 1, &[7u8; 8], Some([0, 0, 1, 1]));
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Command::UpdateContent {
                id,
                w_phys,
                h_phys,
                buffer,
                dirty,
            } => {
                assert_eq!(*id, w.id());
                assert_eq!((*w_phys, *h_phys), (2, 1));
                assert_eq!(&*buffer.as_ref(), &[7u8; 8].as_slice());
                assert_eq!(*dirty, Some([0, 0, 1, 1]));
            }
            other => panic!("意外命令 {other:?}"),
        }
    }

    #[test]
    fn update_content_invalid_inputs_are_discarded_without_commands() {
        let (shared, w) = pair();
        // 长度 != w*h*4 => 丢弃
        w.update_content(2, 1, &[0u8; 7], None);
        // 单边超上限 => 丢弃
        w.update_content(8193, 1, &[0u8; 8], None);
        w.update_content(1, 8193, &[0u8; 8], None);
        assert!(
            pending(&shared).is_empty(),
            "非法参数不得入队（可恢复错误契约：丢弃 + warn-once，不是 Err）"
        );
    }

    #[test]
    fn update_content_zero_size_is_equivalent_to_clear() {
        // FR6：w/h=0 不是错误，语义等价 clear_content。
        let (shared, w) = pair();
        w.update_content(0, 5, &[], None);
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1, "0 尺寸应转译为恰好一条 ClearContent");
        assert!(matches!(cmds[0], Command::ClearContent { id } if id == w.id()));
    }

    #[test]
    fn clear_content_enqueues_its_command() {
        let (shared, w) = pair();
        w.clear_content();
        let cmds = pending(&shared);
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::ClearContent { id } if id == w.id()));
    }

    #[test]
    fn events_are_queued_and_drained_once() {"""

c = c.replace(anchor, new_tests, 1)
with io.open(path, "w", encoding="utf-8", newline="\n") as f:
    f.write(c)
print("handle tests added")
