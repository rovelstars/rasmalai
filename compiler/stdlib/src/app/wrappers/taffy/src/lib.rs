use taffy::prelude::*;

const REC: usize = 40;
const OUT: usize = 32;

#[no_mangle]
pub extern "C" fn taffy_layout(
    nodes: *const u8,
    count: i32,
    root_w: f64,
    root_h: f64,
    out: *mut u8,
) -> i64 {
    if nodes.is_null() || out.is_null() || count <= 0 {
        return -1;
    }
    let n = count as usize;
    let mut tree: TaffyTree<()> = TaffyTree::new();
    let mut ids: Vec<NodeId> = Vec::with_capacity(n);

    for i in 0..n {
        let base = unsafe { nodes.add(i * REC) };
        let read_i = |off: usize| unsafe { (base.add(off) as *const i32).read_unaligned() };
        let read_f = |off: usize| unsafe { (base.add(off) as *const f64).read_unaligned() };
        let parent = read_i(0);
        let kind = read_i(4);
        let w = read_f(8) as f32;
        let h = read_f(16) as f32;
        let pad = read_f(24) as f32;
        let gap = read_f(32) as f32;

        let mut style = Style::default();
        style.flex_direction = if kind == 1 {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        };
        style.size = Size {
            width: if w > 0.0 {
                Dimension::length(w)
            } else {
                Dimension::auto()
            },
            height: if h > 0.0 {
                Dimension::length(h)
            } else {
                Dimension::auto()
            },
        };
        style.padding = Rect {
            left: LengthPercentage::length(pad),
            right: LengthPercentage::length(pad),
            top: LengthPercentage::length(pad),
            bottom: LengthPercentage::length(pad),
        };
        style.gap = Size {
            width: LengthPercentage::length(gap),
            height: LengthPercentage::length(gap),
        };

        let id = match tree.new_leaf(style) {
            Ok(id) => id,
            Err(_) => return -2,
        };
        ids.push(id);
        if parent >= 0 {
            let p = parent as usize;
            if p >= i {
                return -3;
            }
            if tree.add_child(ids[p], id).is_err() {
                return -4;
            }
        }
    }

    let root = ids[0];
    let space = Size {
        width: AvailableSpace::Definite(root_w as f32),
        height: AvailableSpace::Definite(root_h as f32),
    };
    if tree.compute_layout(root, space).is_err() {
        return -5;
    }
    for (i, id) in ids.iter().enumerate() {
        let layout = match tree.layout(*id) {
            Ok(l) => l,
            Err(_) => return -6,
        };
        let at = unsafe { out.add(i * OUT) };
        unsafe {
            (at as *mut f64).write_unaligned(layout.location.x as f64);
            (at.add(8) as *mut f64).write_unaligned(layout.location.y as f64);
            (at.add(16) as *mut f64).write_unaligned(layout.size.width as f64);
            (at.add(24) as *mut f64).write_unaligned(layout.size.height as f64);
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn taffy_abi_version() -> i64 {
    1
}
