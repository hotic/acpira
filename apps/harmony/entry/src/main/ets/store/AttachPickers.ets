import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { fileIo, picker } from '@kit.CoreFileKit';
import { cameraPicker } from '@kit.CameraKit';
import { image } from '@kit.ImageKit';
import { util } from '@kit.ArkTS';
import { Attachment } from '../wire/Wire';
import { Cmp } from '../design/Tokens';

// The system pickers behind the attach beans (apps/ios Chat/Composer.swift AttachPickers, apps/android ui/Composer.kt):
// photos through the photo picker (no permission), the camera through the system camera picker, any file through the
// document picker. Photos are cut to Cmp.imageSide px as JPEG (AttachmentImage.thumbnail) and travel inline as base64;
// files travel whole inside the one message, so the composer checks the draft against MESSAGE_BYTES before it goes.

function base64(buffer: ArrayBuffer): string {
  return new util.Base64Helper().encodeToStringSync(new Uint8Array(buffer));
}

/** The last path segment of a picked uri, decoded */
function nameOf(uri: string, fallback: string): string {
  const last = uri.split('/').pop() ?? '';
  try {
    return last.length > 0 ? decodeURIComponent(last) : fallback;
  } catch (e) {
    return last.length > 0 ? last : fallback;
  }
}

function close(file: fileIo.File | undefined): void {
  if (file === undefined) {
    return;
  }
  try {
    fileIo.closeSync(file);
  } catch (e) {
    // Already gone
  }
}

/** A picked photo as the composer sends it: the longest side cut to imageSide px, JPEG, base64, its thumbnail inline */
async function stageImage(uri: string): Promise<Attachment | undefined> {
  let file: fileIo.File | undefined = undefined;
  try {
    file = fileIo.openSync(uri, fileIo.OpenMode.READ_ONLY);
    const source = image.createImageSource(file.fd);
    const info = source.getImageInfoSync();
    const scale = Math.min(1, Cmp.imageSide / Math.max(info.size.width, info.size.height));
    const size: image.Size = {
      width: Math.max(1, Math.round(info.size.width * scale)), height: Math.max(1, Math.round(info.size.height * scale))
    };
    const map = source.createPixelMapSync({ desiredSize: size });
    const packed = await image.createImagePacker().packToData(map, { format: 'image/jpeg', quality: Cmp.jpegQuality });
    map.release();
    source.release();
    const data = base64(packed);
    const out: Attachment = {
      kind: 'image', blob: data, mimeType: 'image/jpeg', name: nameOf(uri, 'image.jpg'), thumb: `data:image/jpeg;base64,${data}`
    };
    return out;
  } catch (e) {
    return undefined;
  } finally {
    close(file);
  }
}

/** A picked file with its bytes: the engine stages it on the computer and links the staged path */
function stageFile(uri: string): Attachment | undefined {
  let file: fileIo.File | undefined = undefined;
  try {
    file = fileIo.openSync(uri, fileIo.OpenMode.READ_ONLY);
    const size = fileIo.statSync(file.fd).size;
    const buffer = new ArrayBuffer(size);
    fileIo.readSync(file.fd, buffer);
    const out: Attachment = { kind: 'file', uri: uri, name: nameOf(uri, 'file'), data: base64(buffer) };
    return out;
  } catch (e) {
    return undefined;
  } finally {
    close(file);
  }
}

async function stageImages(uris: string[]): Promise<Attachment[]> {
  const out: Attachment[] = [];
  for (const uri of uris) {
    const a = await stageImage(uri);
    if (a !== undefined) {
      out.push(a);
    }
  }
  return out;
}

/** Photos from the system photo picker, up to Cmp.maxPhotos */
export async function pickPhotos(): Promise<Attachment[]> {
  const options = new photoAccessHelper.PhotoSelectOptions();
  options.MIMEType = photoAccessHelper.PhotoViewMIMETypes.IMAGE_TYPE;
  options.maxSelectNumber = Cmp.maxPhotos;
  try {
    const result = await new photoAccessHelper.PhotoViewPicker().select(options);
    return stageImages(result.photoUris);
  } catch (e) {
    return [];
  }
}

/** One photo from the system camera */
export async function takePhoto(context: Context): Promise<Attachment[]> {
  // A device without a camera has no picker for it
  if (canIUse('SystemCapability.Multimedia.Camera.Core')) {
    try {
      const profile = new cameraPicker.PickerProfile();
      const result = await cameraPicker.pick(context, [cameraPicker.PickerMediaType.PHOTO], profile);
      if (result.resultCode === 0 && result.resultUri.length > 0) {
        return stageImages([result.resultUri]);
      }
    } catch (e) {
      return [];
    }
  }
  return [];
}

/** Any files from the system document picker */
export async function pickFiles(context: Context): Promise<Attachment[]> {
  const options = new picker.DocumentSelectOptions();
  options.maxSelectNumber = Cmp.maxPhotos;
  let uris: string[] = [];
  try {
    uris = await new picker.DocumentViewPicker(context).select(options);
  } catch (e) {
    return [];
  }
  const out: Attachment[] = [];
  uris.forEach((uri: string) => {
    const a = stageFile(uri);
    if (a !== undefined) {
      out.push(a);
    }
  });
  return out;
}
