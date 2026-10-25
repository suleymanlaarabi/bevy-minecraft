// Import official model geometry without starting Minecraft or its renderer.
// The input describes factory methods, not a list of hardcoded block handlers.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.builders.LayerDefinition;
import net.minecraft.client.model.geom.builders.MeshDefinition;
import org.joml.*;

class ExtractBlockModels {
    private static final Gson JSON = new Gson();
    private static final Field CUBES;
    private static final Field CHILDREN;
    static {
        try {
            CUBES = ModelPart.class.getDeclaredField("cubes");
            CHILDREN = ModelPart.class.getDeclaredField("children");
            CUBES.setAccessible(true);
            CHILDREN.setAccessible(true);
        } catch (ReflectiveOperationException error) {
            throw new ExceptionInInitializerError(error);
        }
    }

    @SuppressWarnings("unchecked")
    private static void collect(ModelPart part, Matrix4f parent, String path,
                                JsonObject overrides, JsonArray output) throws Exception {
        JsonObject override = overrides.has(path) ? overrides.getAsJsonObject(path) : new JsonObject();
        for (var entry : override.entrySet()) {
            ModelPart.class.getField(entry.getKey()).setFloat(part, entry.getValue().getAsFloat());
        }
        Matrix4f transform = new Matrix4f(parent)
            .translate(part.x / 16, part.y / 16, part.z / 16)
            .rotateZYX(part.zRot, part.yRot, part.xRot)
            .scale(part.xScale, part.yScale, part.zScale);
        Matrix3f normals = new Matrix3f(transform).invert().transpose();
        for (ModelPart.Cube cube : (List<ModelPart.Cube>) CUBES.get(part)) {
            for (var polygon : cube.polygons) {
                JsonObject quad = new JsonObject();
                JsonArray positions = new JsonArray();
                JsonArray uv = new JsonArray();
                for (var vertex : polygon.vertices()) {
                    Vector3f position = transform.transformPosition(
                        new Vector3f(vertex.worldX(), vertex.worldY(), vertex.worldZ()));
                    positions.add(JSON.toJsonTree(new float[]{position.x, position.y, position.z}));
                    uv.add(JSON.toJsonTree(new float[]{vertex.u(), vertex.v()}));
                }
                Vector3f normal = normals.transform(new Vector3f(polygon.normal())).normalize();
                quad.add("positions", positions);
                quad.add("uv", uv);
                quad.add("normal", JSON.toJsonTree(new float[]{normal.x, normal.y, normal.z}));
                quad.addProperty("part", path);
                output.add(quad);
            }
        }
        for (var child : ((Map<String, ModelPart>) CHILDREN.get(part)).entrySet()) {
            collect(child.getValue(), transform, path + "/" + child.getKey(), overrides, output);
        }
    }

    public static void main(String[] args) throws Exception {
        JsonObject requests = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        JsonObject result = new JsonObject();
        for (var entry : requests.entrySet()) {
            JsonObject request = entry.getValue().getAsJsonObject();
            JsonArray arguments = request.has("args") ? request.getAsJsonArray("args") : new JsonArray();
            Class<?>[] types = new Class<?>[arguments.size()];
            Object[] values = new Object[arguments.size()];
            for (int i = 0; i < arguments.size(); i++) {
                types[i] = boolean.class;
                values[i] = arguments.get(i).getAsBoolean();
            }
            Object factory = Class.forName(request.get("class").getAsString())
                .getMethod(request.get("method").getAsString(), types).invoke(null, values);
            LayerDefinition layer = factory instanceof LayerDefinition definition ? definition
                : LayerDefinition.create((MeshDefinition) factory,
                    request.get("width").getAsInt(), request.get("height").getAsInt());
            JsonArray quads = new JsonArray();
            collect(layer.bakeRoot(), new Matrix4f(), "",
                request.has("parts") ? request.getAsJsonObject("parts") : new JsonObject(), quads);
            result.add(entry.getKey(), quads);
        }
        Files.writeString(Path.of(args[1]), JSON.toJson(result));
    }
}
