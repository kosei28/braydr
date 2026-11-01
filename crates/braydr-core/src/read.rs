//! Read position rules (`docs/product-spec.md` 4.5).

/// One message laid out in the timeline, in timeline order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Laid<'a> {
    pub message_id: &'a str,
    pub bottom: f32,
}

/// The newest message whose bottom edge is inside the viewport.
pub fn newest_seen<'a>(laid: &[Laid<'a>], viewport_bottom: f32) -> Option<&'a str> {
    laid.iter()
        .filter(|item| item.bottom <= viewport_bottom)
        .map(|item| item.message_id)
        .max()
}

/// Read positions never move back. IDs are UUIDv7, so string order is time order.
pub fn advance<'a>(current: Option<&'a str>, seen: Option<&'a str>) -> Option<&'a str> {
    match (current, seen) {
        (Some(current), Some(seen)) => Some(current.max(seen)),
        (current, seen) => current.or(seen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn only_messages_with_a_visible_bottom_count() {
        let laid = [
            Laid {
                message_id: "a",
                bottom: 100.,
            },
            Laid {
                message_id: "b",
                bottom: 300.,
            },
            Laid {
                message_id: "c",
                bottom: 900.,
            },
        ];
        assert_eq!(newest_seen(&laid, 400.), Some("b"));
        assert_eq!(newest_seen(&laid, 50.), None);
    }

    proptest! {
        #[test]
        fn never_moves_back(current in "[a-f]{1,4}", seen in "[a-f]{1,4}") {
            let next = advance(Some(&current), Some(&seen)).unwrap();
            prop_assert!(next >= current.as_str());
            prop_assert!(next >= seen.as_str());
        }
    }
}
