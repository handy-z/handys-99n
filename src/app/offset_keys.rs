use handy_sdk::roblox::events::CHAT_KEYS;
use handy_sdk::roblox::offsets::{OffsetKeys, SDK_KEYS};
use handy_sdk::roblox::read::join_keys;

/// Every offset 99n reads (a superset of SDK_KEYS and CHAT_KEYS); offsets missing any of these are rejected.
pub fn required_keys() -> OffsetKeys {
    join_keys(
        &[&SDK_KEYS, &CHAT_KEYS],
        &[
            "DataModel.Workspace",
            "Workspace.CurrentCamera",
            "Camera.CameraSubject",
            "Player.LocalPlayer",
            "GuiObject.Visible",
            "GuiObject.Size",
            "GuiObject.Text",
            "GuiBase2D.AbsolutePosition",
            "Model.PrimaryPart",
            "BasePart.Primitive",
            "Primitive.Position",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_keys_cover_sdk_keys() {
        let keys = required_keys();
        for k in CHAT_KEYS {
            assert!(keys.contains(&k));
        }
        for k in SDK_KEYS {
            assert!(keys.contains(&k));
        }
    }
}
