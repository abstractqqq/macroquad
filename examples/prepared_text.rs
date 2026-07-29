use macroquad::prelude::*;

#[macroquad::main("Prepared text")]
async fn main() {
    let text = "Prepared text is shaped once.\nThis line wraps and reveals by cluster.";
    let emphasized = text.find("shaped once").unwrap();
    let layout = prepare_text_layout(
        text,
        TextLayoutParams {
            font_size: 32,
            max_width: Some(520.0),
            ..Default::default()
        },
        &[TextColorSpan {
            range: emphasized..emphasized + "shaped once".len(),
            color: YELLOW,
        }],
    );
    let mut visible_clusters = 0;
    let mut elapsed = 0.0;

    loop {
        clear_background(BLACK);
        elapsed += get_frame_time();
        while elapsed >= 0.04 && visible_clusters < layout.cluster_count() {
            elapsed -= 0.04;
            visible_clusters += 1;
        }
        draw_text_layout_ex(
            &layout,
            40.0,
            80.0,
            TextLayoutDrawParams {
                visible_clusters: Some(visible_clusters),
                ..Default::default()
            },
        );

        next_frame().await;
    }
}
