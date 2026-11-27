import java.lang.classfile.ClassFile;
import java.lang.classfile.Opcode;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Emits Victim.class (classfile version 52), equivalent to this pseudo-Java:
 *
 *   public Victim(int n) {
 *       super();
 *       int0 = n;                       // istore_0: receiver slot now holds an int (legal per JVMS 4.10.1.2)
 *       for (int i = 0; i < 1_000_000; i++) { Object[] a = new Object[int0]; }  // allocate -> scavenges
 *   }
 *
 * Same shape as R8's n30.<init>(I)V: "dup; istore_0; anewarray".
 */
public class Gen {
    public static void main(String[] args) throws Exception {
        ClassDesc victim = ClassDesc.of("Victim");
        byte[] bytes = ClassFile.of().build(victim, cb -> cb
            .withVersion(ClassFile.JAVA_8_VERSION, 0)
            .withFlags(ClassFile.ACC_PUBLIC | ClassFile.ACC_SUPER)
            .withMethodBody(ConstantDescs.INIT_NAME, MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int),
                ClassFile.ACC_PUBLIC, code -> {
                    var loop = code.newLabel();
                    var done = code.newLabel();
                    code.aload(0)
                        .invokespecial(ConstantDescs.CD_Object, ConstantDescs.INIT_NAME, ConstantDescs.MTD_void)
                        .iload(1)
                        .istore(0)            // overwrite receiver slot 0 with a primitive
                        .iconst_0()
                        .istore(1)            // i = 0
                        .labelBinding(loop)
                        .iload(1)
                        .ldc(1_000_000)
                        .if_icmpge(done)
                        .iload(0)
                        .anewarray(ConstantDescs.CD_Object)   // GC point while slot 0 holds an int
                        .pop()
                        .iinc(1, 1)
                        .goto_(loop)
                        .labelBinding(done)
                        .return_();
                })
            .withMethodBody("main", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_String.arrayType()),
                ClassFile.ACC_PUBLIC | ClassFile.ACC_STATIC, code -> code
                    .new_(victim).dup()
                    .ldc(1025)                // odd int -> low word 0x401 => "Misaligned object"
                    .invokespecial(victim, ConstantDescs.INIT_NAME, MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int))
                    .getstatic(ClassDesc.of("java.lang.System"), "out", ClassDesc.of("java.io.PrintStream"))
                    .ldc("OK: constructor finished")
                    .invokevirtual(ClassDesc.of("java.io.PrintStream"), "println", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_String))
                    .return_()));
        Files.write(Path.of(args.length > 0 ? args[0] : "Victim.class"), bytes);
    }
}
