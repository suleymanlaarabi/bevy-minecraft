// Read official registry sound metadata without starting the client or renderer.
import com.google.gson.*;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.properties.*;
import net.minecraft.sounds.SoundEvent;

class ExtractBlockSounds {
    private static void sound(JsonObject object, String key, SoundEvent sound) {
        object.addProperty(key, sound.location().getPath());
    }
    private static Object type(Block block, Class<?> owner) throws Exception {
        var field = owner.getDeclaredField("type");
        field.setAccessible(true);
        return field.get(block);
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        JsonObject blocks = new JsonObject();
        for (Block block : BuiltInRegistries.BLOCK) {
            var state = block.defaultBlockState();
            SoundType sounds = state.getSoundType();
            JsonObject info = new JsonObject();
            info.addProperty("volume", sounds.volume);
            info.addProperty("pitch", sounds.pitch);
            sound(info, "break", sounds.getBreakSound());
            sound(info, "step", sounds.getStepSound());
            sound(info, "place", sounds.getPlaceSound());
            sound(info, "hit", sounds.getHitSound());
            sound(info, "fall", sounds.getFallSound());
            if (block instanceof DoorBlock door) {
                sound(info, "open", door.type().doorOpen());
                sound(info, "close", door.type().doorClose());
            } else if (block instanceof TrapDoorBlock) {
                var set = (BlockSetType) type(block, TrapDoorBlock.class);
                sound(info, "open", set.trapdoorOpen());
                sound(info, "close", set.trapdoorClose());
            } else if (block instanceof FenceGateBlock) {
                var wood = (WoodType) type(block, FenceGateBlock.class);
                sound(info, "open", wood.fenceGateOpen());
                sound(info, "close", wood.fenceGateClose());
            }
            blocks.add(BuiltInRegistries.BLOCK.getKey(block).getPath(), info);
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(blocks));
    }
}
