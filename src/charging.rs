pub fn charging_colors(percent: u8, brightness: u8, direction: u8) -> [[u8; 3]; 12] {
    let percent = u32::from(percent.min(100));
    let brightness = u32::from(brightness.min(4)) + 1;
    let color = [
        (255 * (100 - percent).min(50) + 25) / 50,
        (255 * percent.min(50) + 25) / 50,
        0,
    ];
    std::array::from_fn(|index| {
        let position = if direction == 1 { 11 - index } else { index };
        let coverage = (percent * 12)
            .saturating_sub(position as u32 * 100)
            .min(100);
        color.map(|channel| ((channel * brightness * coverage + 250) / 500) as u8)
    })
}

#[cfg(test)]
mod tests {
    use super::charging_colors;

    #[test]
    fn empty_half_and_full_charge_match_ring_anchors() {
        assert_eq!(charging_colors(0, 4, 0), [[0; 3]; 12]);
        let half = charging_colors(50, 4, 0);
        assert_eq!(&half[..6], &[[255, 255, 0]; 6]);
        assert_eq!(&half[6..], &[[0; 3]; 6]);
        assert_eq!(charging_colors(100, 4, 0), [[0, 255, 0]; 12]);
        assert_eq!(charging_colors(255, 4, 0), charging_colors(100, 4, 0));
    }

    #[test]
    fn fractional_led_fills_between_whole_segments() {
        let colors = charging_colors(55, 4, 0);
        assert_eq!(&colors[..6], &[[230, 255, 0]; 6]);
        assert_eq!(colors[6], [138, 153, 0]);
        assert_eq!(&colors[7..], &[[0; 3]; 5]);
        assert_eq!(charging_colors(1, 4, 0)[0], [31, 1, 0]);
    }

    #[test]
    fn filled_area_never_decreases_at_any_brightness() {
        for brightness in 0..=4 {
            let mut previous_area = 0;
            for percent in 0..=100 {
                let area: u32 = charging_colors(percent, brightness, 0)
                    .iter()
                    .map(|color| u32::from(*color.iter().max().unwrap()))
                    .sum();
                assert!(
                    area >= previous_area,
                    "area decreased at {percent}% and brightness {brightness}"
                );
                previous_area = area;
            }
            assert_eq!(previous_area, 12 * 51 * (u32::from(brightness) + 1));
        }
    }

    #[test]
    fn hue_moves_from_red_through_yellow_to_green() {
        let mut previous = charging_colors(9, 4, 0)[0];
        assert_eq!(previous, [255, 46, 0]);
        for percent in 10..=100 {
            let color = charging_colors(percent, 4, 0)[0];
            assert!(color[0] <= previous[0]);
            assert!(color[1] >= previous[1]);
            assert_eq!(color[2], 0);
            previous = color;
        }
        assert_eq!(previous, [0, 255, 0]);
    }

    #[test]
    fn direction_reverses_the_entire_ring() {
        for percent in 0..=100 {
            let mut forward = charging_colors(percent, 4, 0);
            forward.reverse();
            assert_eq!(charging_colors(percent, 4, 1), forward);
        }
    }

    #[test]
    fn brightness_uses_five_steps_and_clamps_at_full() {
        for (brightness, channel) in [51, 102, 153, 204, 255].into_iter().enumerate() {
            assert_eq!(
                charging_colors(50, brightness as u8, 0)[0],
                [channel, channel, 0]
            );
        }
        for percent in 0..=100 {
            assert_eq!(
                charging_colors(percent, 255, 0),
                charging_colors(percent, 4, 0)
            );
        }
    }
}
