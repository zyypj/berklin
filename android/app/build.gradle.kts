import org.gradle.api.file.FileSystemOperations
import java.util.Properties
import javax.inject.Inject

plugins {
    id("com.android.application")
}

abstract class CopyUiAssets : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val uiDistDirectory: DirectoryProperty

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val placeholderDirectory: DirectoryProperty

    @get:OutputDirectory
    abstract val assetsRootDirectory: DirectoryProperty

    @get:Inject
    abstract val fileSystem: FileSystemOperations

    @TaskAction
    fun copyUi() {
        val uiDist = uiDistDirectory.get().asFile
        val source = if (uiDist.resolve("index.html").isFile) uiDist else placeholderDirectory.get().asFile
        val target = assetsRootDirectory.get().dir("ui")
        fileSystem.sync {
            from(source)
            into(target)
        }
    }
}

val copyUiAssets = tasks.register<CopyUiAssets>("copyUiAssets") {
    uiDistDirectory.set(rootProject.layout.projectDirectory.dir("../ui/dist"))
    placeholderDirectory.set(layout.projectDirectory.dir("src/main/assets-placeholder"))
}

val releaseKeystorePropertiesFile = rootProject.file("keystore.properties")
val releaseKeystoreProperties = Properties().apply {
    if (releaseKeystorePropertiesFile.isFile) releaseKeystorePropertiesFile.inputStream().use { load(it) }
}
val hasReleaseKeystore = releaseKeystorePropertiesFile.isFile

android {
    namespace = "dev.zyypj.berklin"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.zyypj.berklin"
        minSdk = 26
        targetSdk = 35
        versionCode = providers.gradleProperty("BERKLIN_VERSION_CODE").get().toInt()
        versionName = providers.gradleProperty("BERKLIN_VERSION_NAME").get()
    }

    signingConfigs {
        if (hasReleaseKeystore) {
            create("release") {
                storeFile = file(releaseKeystoreProperties.getProperty("storeFile"))
                storePassword = releaseKeystoreProperties.getProperty("storePassword")
                keyAlias = releaseKeystoreProperties.getProperty("keyAlias")
                keyPassword = releaseKeystoreProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            if (hasReleaseKeystore) signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        buildConfig = true
        resValues = false
    }

    packaging {
        resources {
            excludes += setOf("META-INF/*.version", "META-INF/*.kotlin_module", "kotlin/**", "DebugProbesKt.bin")
        }
    }
}

androidComponents {
    onVariants { variant ->
        variant.sources.assets?.addGeneratedSourceDirectory(copyUiAssets, CopyUiAssets::assetsRootDirectory)
    }
}

dependencies {
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
